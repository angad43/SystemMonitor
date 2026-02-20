slint::include_modules!();

use nvml_wrapper::Nvml;
use slint::{Model, VecModel};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;
use sysinfo::{Components, CpuRefreshKind, MemoryRefreshKind, System};

fn generate_svg_paths(history: &[f32]) -> (String, String) {
    if history.len() < 2 {
        return (String::new(), String::new());
    }
    let max_x = (history.len() - 1) as f32;
    let bottom_y = 280.0_f32;
    let scale = 2.75_f32;

    let y0 = bottom_y - history[0].clamp(0.0, 100.0) * scale;
    let mut line = format!("M 0 {:.2}", y0);

    for (i, &val) in history.iter().enumerate().skip(1) {
        let x = (i as f32 / max_x) * 765.0;
        let y = bottom_y - val.clamp(0.0, 100.0) * scale;
        line.push_str(&format!(" L {:.2} {:.2}", x, y));
    }

    let mut fill = line.clone();
    fill.push_str(&format!(" L 765 {0:.2} L 0 {0:.2} Z", bottom_y));
    (line, fill)
}

fn read_u64(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_str(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[derive(Clone, Debug)]
enum GpuKind {
    Nvidia(u32),
    AmdIntel(PathBuf),
}

fn hwmon_read(card_path: &Path, filename: &str) -> Option<u64> {
    let hwmon_dir = card_path.join("device/hwmon");
    for entry in fs::read_dir(&hwmon_dir).ok()?.flatten() {
        if let Some(v) = read_u64(&entry.path().join(filename)) {
            return Some(v);
        }
    }
    None
}

fn sysfs_temp(card_path: &Path) -> Option<f32> {
    hwmon_read(card_path, "temp1_input").map(|t| t as f32 / 1000.0)
}

fn sysfs_power_w(card_path: &Path) -> Option<f32> {
    hwmon_read(card_path, "power1_average")
        .or_else(|| hwmon_read(card_path, "power1_input"))
        .map(|uw| uw as f32 / 1_000_000.0)
}
fn intel_engine_busy_ns_total(card_path: &Path, card_name: &str) -> Option<u64> {
    let engine_root = card_path.join("device/drm").join(card_name).join("engine");

    let mut total: u64 = 0;
    let mut found = false;
    for entry in fs::read_dir(&engine_root).ok()?.flatten() {
        if let Some(ns) = read_u64(&entry.path().join("busy")) {
            total += ns;
            found = true;
        }
    }
    if found { Some(total) } else { None }
}
fn intel_utilisation(
    card_path: &Path,
    card_name: &str,
    engine_state: &mut HashMap<PathBuf, (u64, Instant)>,
) -> Option<f32> {
    let busy_now = intel_engine_busy_ns_total(card_path, card_name)?;
    let now = Instant::now();

    let util = if let Some((prev_busy, prev_instant)) = engine_state.get(card_path) {
        let elapsed_ns = prev_instant.elapsed().as_nanos() as u64;
        if elapsed_ns == 0 {
            0.0
        } else {
            let delta_busy = busy_now.saturating_sub(*prev_busy);
            (delta_busy as f32 / elapsed_ns as f32 * 100.0).clamp(0.0, 100.0)
        }
    } else {
        0.0
    };

    engine_state.insert(card_path.to_path_buf(), (busy_now, now));
    Some(util)
}
fn amd_utilisation(card_path: &Path) -> Option<f32> {
    read_u64(&card_path.join("device/gpu_busy_percent")).map(|v| v as f32)
}

fn sysfs_freq_mhz(card_path: &Path, card_name: &str) -> Option<u64> {
    // Intel: gt_cur_freq_mhz
    if let Some(v) = read_u64(
        &card_path
            .join("device/drm")
            .join(card_name)
            .join("gt_cur_freq_mhz"),
    )
    .or_else(|| read_u64(&card_path.join("device/gt_cur_freq_mhz")))
    {
        return Some(v);
    }
    if let Ok(content) = fs::read_to_string(card_path.join("device/pp_dpm_sclk")) {
        for line in content.lines() {
            if line.contains('*') {
                let freq_str = line
                    .split(':')
                    .nth(1)
                    .unwrap_or("")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches("Mhz")
                    .trim_end_matches("MHz");
                if let Ok(mhz) = freq_str.parse::<u64>() {
                    return Some(mhz);
                }
            }
        }
    }
    hwmon_read(card_path, "freq1_input").map(|hz| hz / 1_000_000)
}
// main
fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let ui_handle = ui.as_weak();

    let mut sys = System::new_all();
    let mut components = Components::new_with_refreshed_list();
    let nvml = Nvml::init().ok();

    // GPU discovery
    let mut gpu_kinds: Vec<(String, GpuKind)> = Vec::new();

    if let Some(ref n) = nvml {
        if let Ok(count) = n.device_count() {
            for i in 0..count {
                if let Ok(dev) = n.device_by_index(i) {
                    let name = dev.name().unwrap_or_else(|_| format!("NVIDIA GPU {}", i));
                    gpu_kinds.push((name, GpuKind::Nvidia(i)));
                }
            }
        }
    }

    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        let mut drm_cards: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                name.starts_with("card") && !name.contains('-')
            })
            .collect();
        drm_cards.sort();

        for card_path in drm_cards {
            let vendor = read_str(&card_path.join("device/vendor")).to_lowercase();
            if vendor.contains("0x10de") {
                continue;
            }

            let name = {
                let product = read_str(&card_path.join("device/product_name"));
                if !product.is_empty() {
                    product
                } else if vendor.contains("0x1002") {
                    let hwmon_name = card_path
                        .join("device/hwmon")
                        .read_dir()
                        .ok()
                        .and_then(|mut d| d.next())
                        .and_then(|e| e.ok())
                        .map(|e| read_str(&e.path().join("name")))
                        .unwrap_or_default();
                    if !hwmon_name.is_empty() && hwmon_name != "amdgpu" {
                        format!("AMD {} (amdgpu)", hwmon_name)
                    } else {
                        "AMD Radeon Graphics".to_string()
                    }
                } else if vendor.contains("0x8086") {
                    "Intel Integrated Graphics (i915)".to_string()
                } else {
                    format!(
                        "GPU ({})",
                        card_path.file_name().unwrap_or_default().to_string_lossy()
                    )
                }
            };

            gpu_kinds.push((name, GpuKind::AmdIntel(card_path)));
        }
    }
    // Build Slint model
    let gpu_models = Rc::new(VecModel::<GpuData>::default());
    let mut gpu_histories: Vec<Vec<f32>> = Vec::new();
    for (name, _) in &gpu_kinds {
        let mut d = GpuData::default();
        d.name = name.clone().into();
        d.usage = "0%".into();
        d.temp = "--°C".into();
        d.freq = "--".into();
        d.vram_total = "--".into();
        d.vram_used = "--".into();
        d.wattage = "--".into();
        gpu_models.push(d);
        gpu_histories.push(vec![0.0_f32; 100]);
    }
    ui.set_gpus(gpu_models.clone().into());
    // CPU one-time setup
    sys.refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage().with_frequency());
    if let Some(cpu) = sys.cpus().first() {
        ui.set_processor_name(cpu.brand().trim().into());
        ui.set_base_speed(format!("{:.2} GHz", cpu.frequency() as f32 / 1000.0).into());
    }
    ui.set_cpu_cores(sys.physical_core_count().unwrap_or(0) as i32);
    ui.set_threads(sys.cpus().len() as i32);
    let cpu_history = Rc::new(VecModel::<f32>::from(vec![0.0_f32; 100]));
    let mem_history = Rc::new(VecModel::<f32>::from(vec![0.0_f32; 100]));
    let mut engine_state: HashMap<PathBuf, (u64, Instant)> = HashMap::new();

    // 1-second update timer
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(1000),
        move || {
            let ui = match ui_handle.upgrade() {
                Some(u) => u,
                None => return,
            };

            const GB: f32 = 1024.0 * 1024.0 * 1024.0;
            const MB: f32 = 1024.0 * 1024.0;
            // CPU
            sys.refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage().with_frequency());
            components.refresh();

            let cpu_usage = sys.global_cpu_info().cpu_usage();

            if let Some(cpu) = sys.cpus().first() {
                ui.set_cpu_freq(format!("{:.2} GHz", cpu.frequency() as f32 / 1000.0).into());
            }
            ui.set_cpu_usage(format!("{:.1}%", cpu_usage).into());

            let mut max_temp: f32 = 0.0;
            for comp in &components {
                let label = comp.label().to_uppercase();
                if label.contains("CPU")
                    || label.contains("PACKAGE")
                    || label.contains("CORE")
                    || label.contains("TCTL")
                    || label.contains("TCCD")
                {
                    if comp.temperature() > max_temp {
                        max_temp = comp.temperature();
                    }
                }
            }
            ui.set_cpu_temp(if max_temp > 0.0 {
                format!("{:.0}°C", max_temp).into()
            } else {
                "--°C".into()
            });

            {
                let mut h: Vec<f32> = cpu_history.iter().collect();
                h.remove(0);
                h.push(cpu_usage);
                for (i, v) in h.iter().enumerate() {
                    cpu_history.set_row_data(i, *v);
                }
                let (l, f) = generate_svg_paths(&h);
                ui.set_usage_line_data(l.into());
                ui.set_usage_fill_data(f.into());
            }
            // Memory
            sys.refresh_memory_specifics(MemoryRefreshKind::new().with_ram().with_swap());

            let ram_total = sys.total_memory() as f32;
            let ram_used = sys.used_memory() as f32;
            let ram_avail = sys.available_memory() as f32;
            let ram_free = sys.free_memory() as f32;
            let ram_cached = (ram_avail - ram_free).max(0.0);

            ui.set_ram_total(format!("{:.2} GB", ram_total / GB).into());
            ui.set_ram_active(format!("{:.2} GB", ram_used / GB).into());
            ui.set_ram_available(format!("{:.2} GB", ram_avail / GB).into());
            ui.set_ram_free(format!("{:.2} GB", ram_free / GB).into());
            ui.set_ram_cached(format!("{:.2} GB", ram_cached / GB).into());
            ui.set_swap_total(format!("{:.2} GB", sys.total_swap() as f32 / GB).into());
            ui.set_swap_used(format!("{:.2} GB", sys.used_swap() as f32 / GB).into());
            ui.set_swap_free(format!("{:.2} GB", sys.free_swap() as f32 / GB).into());

            let mem_pct = if ram_total > 0.0 {
                (ram_used / ram_total) * 100.0
            } else {
                0.0
            };
            {
                let mut h: Vec<f32> = mem_history.iter().collect();
                h.remove(0);
                h.push(mem_pct);
                for (i, v) in h.iter().enumerate() {
                    mem_history.set_row_data(i, *v);
                }
                let (l, f) = generate_svg_paths(&h);
                ui.set_mem_line_data(l.into());
                ui.set_mem_fill_data(f.into());
            }
            // GPUs
            for (idx, (_, kind)) in gpu_kinds.iter().enumerate() {
                if idx >= gpu_models.row_count() {
                    break;
                }
                let mut g = gpu_models.row_data(idx).unwrap();
                let mut util: f32 = 0.0;

                match kind {
                    GpuKind::Nvidia(dev_idx) => {
                        if let Some(ref n) = nvml {
                            if let Ok(dev) = n.device_by_index(*dev_idx) {
                                if let Ok(rates) = dev.utilization_rates() {
                                    util = rates.gpu as f32;
                                }
                                g.temp = dev
                                    .temperature(
                                        nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu,
                                    )
                                    .map(|t| format!("{}°C", t))
                                    .unwrap_or_else(|_| "--°C".into())
                                    .into();
                                g.freq = dev
                                    .clock_info(
                                        nvml_wrapper::enum_wrappers::device::Clock::Graphics,
                                    )
                                    .map(|mhz| format!("{} MHz", mhz))
                                    .unwrap_or_else(|_| "--".into())
                                    .into();
                                g.wattage = dev
                                    .power_usage()
                                    .map(|mw| format!("{:.1} W", mw as f32 / 1000.0))
                                    .unwrap_or_else(|_| "--".into())
                                    .into();
                                if let Ok(mem) = dev.memory_info() {
                                    g.vram_total =
                                        format!("{:.1} GB", mem.total as f32 / GB).into();
                                    g.vram_used = format!("{:.0} MB", mem.used as f32 / MB).into();
                                }
                            }
                        }
                    }

                    GpuKind::AmdIntel(card_path) => {
                        let card_name = card_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();

                        let vendor = read_str(&card_path.join("device/vendor")).to_lowercase();

                        util = if vendor.contains("0x1002") {
                            // AMD: gpu_busy_percent is a direct hardware counter — accurate
                            amd_utilisation(card_path).unwrap_or(0.0)
                        } else {
                            // Intel (and unknown vendors): engine busy-time delta.
                            // We deliberately DO NOT fall back to frequency ratio —
                            // idle iGPUs sit at a non-zero base clock which gives
                            // a misleading ~30-40% reading with that heuristic.
                            intel_utilisation(card_path, &card_name, &mut engine_state)
                                .unwrap_or(0.0)
                        };

                        g.freq = sysfs_freq_mhz(card_path, &card_name)
                            .map(|mhz| format!("{} MHz", mhz))
                            .unwrap_or_else(|| "--".into())
                            .into();

                        g.temp = sysfs_temp(card_path)
                            .map(|t| format!("{:.0}°C", t))
                            .unwrap_or_else(|| "--°C".into())
                            .into();

                        g.wattage = sysfs_power_w(card_path)
                            .map(|w| format!("{:.1} W", w))
                            .unwrap_or_else(|| "--".into())
                            .into();

                        let vram_tot =
                            read_u64(&card_path.join("device/mem_info_vram_total")).unwrap_or(0);
                        let vram_used_bytes =
                            read_u64(&card_path.join("device/mem_info_vram_used")).unwrap_or(0);
                        if vram_tot > 0 {
                            g.vram_total = format!("{:.1} GB", vram_tot as f32 / GB).into();
                            g.vram_used = format!("{:.0} MB", vram_used_bytes as f32 / MB).into();
                        } else {
                            g.vram_total = "Shared".into();
                            g.vram_used = "N/A".into();
                        }
                    }
                }

                util = util.clamp(0.0, 100.0);
                g.usage = format!("{:.0}%", util).into();

                gpu_histories[idx].remove(0);
                gpu_histories[idx].push(util);
                let (l, f) = generate_svg_paths(&gpu_histories[idx]);
                g.line_data = l.into();
                g.fill_data = f.into();

                gpu_models.set_row_data(idx, g);
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
