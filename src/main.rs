slint::include_modules!();

use nvml_wrapper::Nvml;
use slint::{Model, VecModel};
use std::fs;
use std::path::Path;
use std::rc::Rc;
use sysinfo::{Components, CpuRefreshKind, MemoryRefreshKind, System};

// --- Helper: Format Graph Data ---
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

// --- Helpers: Safe File Reading ---
fn read_file_u64(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_file_str(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let ui_handle = ui.as_weak();

    let mut sys = System::new_all();
    let mut components = Components::new_with_refreshed_list();
    let nvml = Nvml::init().ok();

    // --- 1. DISCOVERY PHASE ---
    let mut gpu_candidates: Vec<(String, u8, String)> = Vec::new();

    // A. NVIDIA (NVML)
    if let Some(ref n) = nvml {
        if let Ok(count) = n.device_count() {
            for i in 0..count {
                if let Ok(dev) = n.device_by_index(i) {
                    let name = dev.name().unwrap_or(format!("NVIDIA GPU {}", i));
                    gpu_candidates.push((name, 0, i.to_string()));
                }
            }
        }
    }

    // B. AMD/Intel (SysFS)
    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let path = entry.path();
            let fname = path.file_name().unwrap_or_default().to_string_lossy();
            if fname.starts_with("card") && !fname.contains("-") {
                let vendor_hex = read_file_str(&path.join("device/vendor"));

                // Skip NVIDIA (handled by NVML)
                if !vendor_hex.contains("0x10de") {
                    let mut name = read_file_str(&path.join("device/product_name"));
                    if name.is_empty() {
                        if vendor_hex.contains("0x1002") {
                            name = "AMD Radeon Graphics".into();
                        } else if vendor_hex.contains("0x8086") {
                            name = "Intel HD/UHD Graphics".into();
                        } else {
                            name = format!("GPU ({})", fname);
                        }
                    }
                    gpu_candidates.push((name, 1, path.to_string_lossy().to_string()));
                }
            }
        }
    }

    // --- 2. SETUP MODELS ---
    let gpu_models = Rc::new(VecModel::<GpuData>::default());
    let mut gpu_histories: Vec<Vec<f32>> = Vec::new();

    for (name, _, _) in &gpu_candidates {
        let mut d = GpuData::default();
        d.name = name.clone().into();
        gpu_models.push(d);
        gpu_histories.push(vec![0.0; 100]);
    }
    ui.set_gpus(gpu_models.clone().into());

    let cpu_history = Rc::new(VecModel::<f32>::from(vec![0.0; 100]));
    let mem_history = Rc::new(VecModel::<f32>::from(vec![0.0; 100]));

    sys.refresh_cpu();
    if let Some(cpu) = sys.cpus().first() {
        ui.set_processor_name(cpu.brand().trim().into());
        ui.set_cpu_cores(sys.physical_core_count().unwrap_or(0) as i32);
        ui.set_threads(sys.cpus().len() as i32);
    }

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(1000),
        move || {
            let ui = match ui_handle.upgrade() {
                Some(u) => u,
                None => return,
            };
            let gb = 1024.0 * 1024.0 * 1024.0;
            let mb = 1024.0 * 1024.0;

            sys.refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage().with_frequency());
            sys.refresh_memory_specifics(MemoryRefreshKind::new().with_ram());
            components.refresh();

            // CPU Stats
            if let Some(cpu) = sys.cpus().first() {
                ui.set_cpu_freq(format!("{:.2} GHz", cpu.frequency() as f32 / 1000.0).into());
            }
            let cpu_usage = sys.global_cpu_info().cpu_usage();
            ui.set_cpu_usage(format!("{:.1}%", cpu_usage).into());

            // CPU Temp
            let mut max_temp: f32 = 0.0;
            for component in &components {
                let label = component.label().to_uppercase();
                if label.contains("CPU")
                    || label.contains("PACKAGE")
                    || label.contains("CORE")
                    || label.contains("TCTL")
                {
                    let t = component.temperature();
                    if t > max_temp {
                        max_temp = t;
                    }
                }
            }
            ui.set_cpu_temp(format!("{:.0}°C", max_temp).into());

            cpu_history.remove(0);
            cpu_history.push(cpu_usage);
            let (c_l, c_f) = generate_svg_paths(&cpu_history.iter().collect::<Vec<f32>>());
            ui.set_usage_line_data(c_l.into());
            ui.set_usage_fill_data(c_f.into());

            // Memory
            let total = sys.total_memory() as f32;
            let used = sys.used_memory() as f32;
            ui.set_ram_total(format!("{:.2} GB", total / gb).into());
            ui.set_ram_active(format!("{:.2} GB", used / gb).into());
            ui.set_ram_available(format!("{:.2} GB", sys.available_memory() as f32 / gb).into());
            ui.set_ram_cached(
                format!(
                    "{:.2} GB",
                    (sys.available_memory() - sys.free_memory()) as f32 / gb
                )
                .into(),
            );
            ui.set_ram_free(format!("{:.2} GB", sys.free_memory() as f32 / gb).into());

            let m_pct = (used / total) * 100.0;
            mem_history.remove(0);
            mem_history.push(m_pct);
            let (m_l, m_f) = generate_svg_paths(&mem_history.iter().collect::<Vec<f32>>());
            ui.set_mem_line_data(m_l.into());
            ui.set_mem_fill_data(m_f.into());

            // --- GPU UPDATE LOOP ---
            for (idx, (_, g_type, identifier)) in gpu_candidates.iter().enumerate() {
                if idx >= gpu_models.row_count() {
                    break;
                }
                let mut g_data = gpu_models.row_data(idx).unwrap();
                let mut util = 0.0;

                if *g_type == 0 {
                    // NVIDIA (NVML)
                    if let Some(ref n) = nvml {
                        if let Ok(dev) = n.device_by_index(identifier.parse().unwrap_or(0)) {
                            if let Ok(rates) = dev.utilization_rates() {
                                let gpu_load = rates.gpu as f32;
                                let mem_load = rates.memory as f32;
                                util = gpu_load.max(mem_load);
                            }
                            let last_val = gpu_histories[idx].last().unwrap_or(&0.0);
                            util = (util * 0.4) + (last_val * 0.6);

                            g_data.temp = format!(
                                "{}°C",
                                dev.temperature(
                                    nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu
                                )
                                .unwrap_or(0)
                            )
                            .into();
                            g_data.wattage =
                                format!("{:.1} W", dev.power_usage().unwrap_or(0) as f32 / 1000.0)
                                    .into();
                            g_data.freq = format!(
                                "{} MHz",
                                dev.clock_info(
                                    nvml_wrapper::enum_wrappers::device::Clock::Graphics
                                )
                                .unwrap_or(0)
                            )
                            .into();
                            if let Ok(m) = dev.memory_info() {
                                g_data.vram_total = format!("{:.1} GB", m.total as f32 / gb).into();
                                g_data.vram_used = format!("{:.0} MB", m.used as f32 / mb).into();
                            }
                        }
                    }
                } else {
                    let path = Path::new(identifier);
                    let card_name = path.file_name().unwrap_or_default().to_string_lossy();

                    // 1. UTILIZATION
                    if let Some(busy) = read_file_u64(&path.join("device/gpu_busy_percent")) {
                        util = busy as f32;
                    } else {
                        // Intel Freq Ratio Fallback
                        let gt_max = read_file_u64(
                            &path.join(format!("device/drm/{}/gt_max_freq_mhz", card_name)),
                        )
                        .unwrap_or(1);
                        let gt_cur = read_file_u64(
                            &path.join(format!("device/drm/{}/gt_cur_freq_mhz", card_name)),
                        )
                        .unwrap_or(0);
                        if gt_cur > 0 {
                            util = (gt_cur as f32 / gt_max as f32) * 100.0;
                        }
                    }

                    // 2. FREQUENCY (Improved AMD + Intel Support)
                    let mut found_freq = false;

                    // Try Intel path first
                    if let Some(mhz) = read_file_u64(
                        &path.join(format!("device/drm/{}/gt_cur_freq_mhz", card_name)),
                    ) {
                        g_data.freq = format!("{} MHz", mhz).into();
                        found_freq = true;
                    }

                    // Try AMD PowerPlay path (Format: "0: 300Mhz *")
                    if !found_freq {
                        if let Ok(content) = fs::read_to_string(path.join("device/pp_dpm_sclk")) {
                            for line in content.lines() {
                                if line.contains('*') {
                                    // Extracts "1200" from "1: 1200Mhz *"
                                    let freq = line
                                        .split(':')
                                        .nth(1)
                                        .unwrap_or("")
                                        .split('M')
                                        .next()
                                        .unwrap_or("")
                                        .trim();
                                    g_data.freq = format!("{} MHz", freq).into();
                                    found_freq = true;
                                    break;
                                }
                            }
                        }
                    }

                    // Final Fallback: Check hwmon for freq1_input (common on newer AMD/Intel)
                    if !found_freq {
                        let hwmon_dir = path.join("device/hwmon");
                        if let Ok(entries) = fs::read_dir(&hwmon_dir) {
                            for entry in entries.flatten() {
                                if let Some(hertz) =
                                    read_file_u64(&entry.path().join("freq1_input"))
                                {
                                    g_data.freq = format!("{} MHz", hertz / 1_000_000).into(); // hertz to MHz
                                    found_freq = true;
                                    break;
                                }
                            }
                        }
                    }

                    if !found_freq {
                        g_data.freq = "N/A".into();
                    }
                    // 3. VRAM
                    let vram_tot =
                        read_file_u64(&path.join("device/mem_info_vram_total")).unwrap_or(0);
                    let vram_used =
                        read_file_u64(&path.join("device/mem_info_vram_used")).unwrap_or(0);
                    if vram_tot > 0 {
                        g_data.vram_total = format!("{:.1} GB", vram_tot as f32 / gb).into();
                        g_data.vram_used = format!("{:.0} MB", vram_used as f32 / mb).into();
                    } else {
                        g_data.vram_total = "Shared".into();
                        g_data.vram_used = "N/A".into();
                    }

                    // 4. TEMP
                    let hwmon_dir = path.join("device/hwmon");
                    if let Ok(entries) = fs::read_dir(&hwmon_dir) {
                        for entry in entries.flatten() {
                            if let Some(t) = read_file_u64(&entry.path().join("temp1_input")) {
                                g_data.temp = format!("{:.0}°C", t as f32 / 1000.0).into();
                                break;
                            }
                        }
                    }
                }

                g_data.usage = format!("{:.0}%", util).into();
                gpu_histories[idx].remove(0);
                gpu_histories[idx].push(util);
                let (l, f) = generate_svg_paths(&gpu_histories[idx]);
                g_data.line_data = l.into();
                g_data.fill_data = f.into();
                gpu_models.set_row_data(idx, g_data);
            }

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
