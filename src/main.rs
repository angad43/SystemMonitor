slint::include_modules!();
use slint::{Model, VecModel};
use std::rc::Rc;
use sysinfo::{CpuRefreshKind, ProcessRefreshKind, RefreshKind, System};

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

    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_processes(ProcessRefreshKind::new()),
    );

    sys.refresh_cpu();
    let usage_history = Rc::new(VecModel::<f32>::from(vec![0.0; 100]));
    if let Some(cpu) = sys.cpus().first() {
        let brand = cpu.brand().trim();
        ui.set_processor_name(brand.into());
        let base_speed = if let Some(pos) = brand.find("GHz") {
            let start = brand[..pos].rfind(' ').map(|i| i + 1).unwrap_or(0);
            format!("{} GHz", brand[start..pos].trim())
        } else {
            format!("{:.2} GHz", cpu.frequency() as f32 / 1000.0)
        };
        ui.set_base_speed(base_speed.into());
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

            sys.refresh_cpu();
            sys.refresh_processes();
            let mut freq = sys.global_cpu_info().frequency();
            if freq == 0 {
                freq = sys.cpus().first().map(|c| c.frequency()).unwrap_or(0);
            }
            ui.set_cpu_freq(format!("{:.2} GHz", freq as f32 / 1000.0).into());
            let usage = sys.global_cpu_info().cpu_usage();
            ui.set_cpu_usage(format!("{:.1}%", usage).into());
            usage_history.remove(0);
            usage_history.push(usage);
            let history_vec: Vec<f32> = (0..usage_history.row_count())
                .map(|i| usage_history.row_data(i).unwrap())
                .collect();
            let (line_data, fill_data) = generate_svg_paths(&history_vec);
            ui.set_usage_line_data(line_data.into());
            ui.set_usage_fill_data(fill_data.into());
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
