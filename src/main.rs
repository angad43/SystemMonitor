slint::include_modules!();

use nvml_wrapper::Nvml;
use slint::{Model, VecModel};
use std::rc::Rc;
use sysinfo::{Components, CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, System};

fn generate_svg_paths(history: &[f32]) -> (String, String) {
    if history.len() < 2 {
        return (String::new(), String::new());
    }
    let max_x = history.len() - 1;
    let bottom_y = 280.0;
    let scale = 2.75;
    let y0 = bottom_y - (history[0].clamp(0.0, 100.0) * scale);
    let mut line = format!("M 0 {:.2}", y0);
    for (i, &val) in history.iter().enumerate().skip(1) {
        let x = (i as f32 / max_x as f32) * 765.0;
        let y = bottom_y - (val.clamp(0.0, 100.0) * scale);
        line.push_str(&format!(" L {:.2} {:.2}", x, y));
    }
    let mut fill = line.clone();
    fill.push_str(&format!(" L 765 {0:.2} L 0 {0:.2} Z", bottom_y));

    (line, fill)
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let ui_handle = ui.as_weak();

    let mut sys = System::new_all();
    let mut components = Components::new_with_refreshed_list();
    // Initialize NVML for NVIDIA GPUs
    let nvml = Nvml::init().ok();

    sys.refresh_cpu();
    std::thread::sleep(std::time::Duration::from_millis(200));
    sys.refresh_cpu();

    let cpu_history = Rc::new(VecModel::<f32>::from(vec![0.0; 100]));
    let mem_history = Rc::new(VecModel::<f32>::from(vec![0.0; 100]));
    let gpu_history = Rc::new(VecModel::<f32>::from(vec![0.0; 100]));

    if let Some(cpu) = sys.cpus().first() {
        ui.set_processor_name(cpu.brand().trim().into());
        ui.set_base_speed(format!("{:.2} GHz", cpu.frequency() as f32 / 1000.0).into());
    }
    ui.set_cpu_cores(sys.physical_core_count().unwrap_or(0) as i32);
    ui.set_threads(sys.cpus().len() as i32);
    ui.set_max_speed("5.0 GHz".into());

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(1000),
        move || {
            let ui = match ui_handle.upgrade() {
                Some(ui) => ui,
                None => return,
            };

            sys.refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage().with_frequency());
            sys.refresh_memory_specifics(MemoryRefreshKind::everything());
            sys.refresh_processes_specifics(ProcessRefreshKind::new());
            components.refresh_list();
            components.refresh();

            let gb = 1024.0 * 1024.0 * 1024.0;

            // --- CPU Temperature Detection ---
            let mut best_temp = 0.0;
            let mut highest_score = -1;
            for component in &components {
                let label = component.label().to_uppercase();
                let mut current_score = 0;

                if label.contains("TCTL") || label.contains("TDIE") {
                    current_score = 100;
                } else if label.contains("PACKAGE") {
                    current_score = 90;
                } else if label.contains("CPU") && !label.contains("GPU") {
                    current_score = 80;
                } else if label.contains("CORE") {
                    current_score = 70;
                }

                if current_score > highest_score {
                    highest_score = current_score;
                    best_temp = component.temperature();
                } else if current_score == highest_score && highest_score != -1 {
                    best_temp = best_temp.max(component.temperature());
                }
            }

            if highest_score != -1 {
                ui.set_cpu_temp(format!("{:.0}°C", best_temp).into());
            } else {
                ui.set_cpu_temp("--°C".into());
            }

            // --- CPU Stats ---
            let mut freq_mhz = sys.global_cpu_info().frequency();
            if freq_mhz == 0 {
                freq_mhz = sys.cpus().first().map(|c| c.frequency()).unwrap_or(0);
            }
            ui.set_cpu_freq(format!("{:.2} GHz", freq_mhz as f32 / 1000.0).into());

            let usage = sys.global_cpu_info().cpu_usage();
            ui.set_cpu_usage(format!("{:.1}%", usage).into());
            cpu_history.remove(0);
            cpu_history.push(usage);

            let (cpu_line, cpu_fill) =
                generate_svg_paths(&cpu_history.iter().collect::<Vec<f32>>());
            ui.set_usage_line_data(cpu_line.into());
            ui.set_usage_fill_data(cpu_fill.into());

            // --- GPU Stats ---
            let mut g_usage = 0.0;
            let mut g_temp = 0.0;
            let mut g_name = String::from("Integrated Graphics");
            if let Some(ref n) = nvml {
                if let Ok(dev) = n.device_by_index(0) {
                    g_name = dev.name().unwrap_or(g_name);
                    g_temp = dev
                        .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
                        .unwrap_or(0) as f32;
                    g_usage = dev.utilization_rates().map(|u| u.gpu).unwrap_or(0) as f32;
                    ui.set_gpu_wattage(
                        format!("{:.2} W", dev.power_usage().unwrap_or(0) as f32 / 1000.0).into(),
                    );
                    ui.set_gpu_freq(
                        format!(
                            "{} MHz",
                            dev.clock_info(nvml_wrapper::enum_wrappers::device::Clock::Graphics)
                                .unwrap_or(0)
                        )
                        .into(),
                    );

                    let m = dev.memory_info().unwrap();
                    ui.set_gpu_vram_total(format!("{:.1} GB", m.total as f32 / gb).into());
                    ui.set_gpu_vram_used(
                        format!("{:.0} MB", m.used as f32 / (1024.0 * 1024.0)).into(),
                    );
                }
            } else {
                // Priority 2: AMD/Intel Fallback via scored components
                for comp in &components {
                    let label = comp.label().to_uppercase();
                    if label.contains("GPU") || label.contains("AMDGPU") || label.contains("INTEL")
                    {
                        g_temp = comp.temperature();
                        g_name = label;
                    }
                }
            }

            ui.set_gpu_name(g_name.into());
            ui.set_gpu_temp(format!("{:.0}°C", g_temp).into());
            ui.set_gpu_usage(format!("{:.1}%", g_usage).into());

            gpu_history.remove(0);
            gpu_history.push(g_usage);
            let (g_line, g_fill) = generate_svg_paths(&gpu_history.iter().collect::<Vec<f32>>());
            ui.set_gpu_line_data(g_line.into());
            ui.set_gpu_fill_data(g_fill.into());

            // --- Memory Stats ---
            let total = sys.total_memory() as f32;
            let used = sys.used_memory() as f32;
            let available = sys.available_memory() as f32;
            let free = sys.free_memory() as f32;
            let cached_val = (available - free).max(0.0);

            ui.set_ram_total(format!("{:.2} GB", total / gb).into());
            ui.set_ram_free(format!("{:.2} GB", free / gb).into());
            ui.set_ram_available(format!("{:.2} GB", available / gb).into());
            ui.set_ram_cached(format!("{:.2} GB", cached_val / gb).into());
            ui.set_ram_active(format!("{:.2} GB", used / gb).into());

            ui.set_swap_total(format!("{:.2} GB", sys.total_swap() as f32 / gb).into());
            ui.set_swap_cache(format!("{:.2} GB", sys.used_swap() as f32 / gb).into());

            let mem_usage_pct = (used / total) * 100.0;
            mem_history.remove(0);
            mem_history.push(mem_usage_pct);

            let (mem_line, mem_fill) =
                generate_svg_paths(&mem_history.iter().collect::<Vec<f32>>());
            ui.set_mem_line_data(mem_line.into());
            ui.set_mem_fill_data(mem_fill.into());

            // --- System Stats ---
            ui.set_processes(sys.processes().len() as i32);
            let uptime = System::uptime();
            ui.set_uptime(
                format!(
                    "{:02}:{:02}:{:02}",
                    uptime / 3600,
                    (uptime % 3600) / 60,
                    uptime % 60
                )
                .into(),
            );
        },
    );

    ui.run()
}
