mod app;
mod capture;
mod config;
mod engine;
mod icons;
mod sampler;
mod theme;
mod tray;
mod wled;
mod worker;

use std::time::Duration;

fn headless(_cfg_path: Option<String>) -> ! {
    let cfg = config::Config::load();
    let pipeline = worker::Pipeline::start(&cfg);
    // keep the token in the config file once the portal handed one out
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let latest = pipeline.shared.config();
        if latest.settings.restore_token != cfg.settings.restore_token {
            latest.save();
        }
    }
}

/// A small gradient square so the window has a proper icon.
fn window_icon() -> egui::IconData {
    let size = 64u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 / (size - 1) as f32;
            let fy = y as f32 / (size - 1) as f32;
            let dx = fx - 0.5;
            let dy = fy - 0.5;
            let dist = (dx * dx + dy * dy).sqrt();
            let inside = dist < 0.46;
            let o = ((y * size + x) * 4) as usize;
            if inside {
                let r = (70.0 + 150.0 * fy) as u8;
                let g = (120.0 + 60.0 * (1.0 - fx)) as u8;
                let b = 255.0f32.min(200.0 + 60.0 * fx) as u8;
                rgba[o] = r;
                rgba[o + 1] = g;
                rgba[o + 2] = b;
                rgba[o + 3] = 255;
            }
        }
    }
    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--headless") {
        let path = args
            .iter()
            .position(|a| a == "--config")
            .and_then(|i| args.get(i + 1).cloned());
        headless(path);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 880.0])
            .with_min_inner_size([980.0, 640.0])
            .with_icon(window_icon())
            .with_title("WLED Ambilight"),
        ..Default::default()
    };
    eframe::run_native(
        "wled-ambilight",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
