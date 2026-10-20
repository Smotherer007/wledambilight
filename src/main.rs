// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

// No console window next to the GUI on Windows (release builds).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! WLED Ambilight - screen capture to WLED, on Linux, Windows and macOS.
//!
//! This program is free software: you can redistribute it and/or modify it
//! under the terms of the GNU General Public License version 3 as published by
//! the Free Software Foundation. It is distributed WITHOUT ANY WARRANTY; see
//! the LICENSE file for details.

mod app;
mod capture;
mod config;
mod engine;
mod i18n;
mod icons;
mod sampler;
mod theme;
mod tray;
mod wled;
mod worker;

use std::time::Duration;

/// Reverse-DNS id: Wayland app id, desktop file, Flatpak, icon name.
pub const APP_ID: &str = "io.github.smotherer007.WledAmbilight";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
WLED Ambilight - screen capture to WLED

Usage: wled-ambilight [options]

Options:
  --minimized        start with the window minimised (autostart with tray)
  --headless         run without a window (servers, no tray)
  --config <file>    use this config file instead of the default
  -V, --version      print the version
  -h, --help         print this help
";

fn headless() -> ! {
    let mut cfg = config::Config::load();
    i18n::init(&cfg.settings.language);
    if cfg.devices.is_empty() {
        eprintln!(
            "no devices configured in {} - start the GUI once to set them up",
            config::config_path().display()
        );
    }
    let pipeline = worker::Pipeline::start(&cfg);
    // keep the portal token in the config file once the portal handed one out
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let token = pipeline.shared.restore_token();
        if token.is_some() && token != cfg.settings.restore_token {
            cfg.settings.restore_token = token;
            cfg.save();
        }
    }
}

fn window_icon() -> egui::IconData {
    egui::IconData {
        rgba: include_bytes!("../assets/icons/app-64.rgba").to_vec(),
        width: 64,
        height: 64,
    }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut run_headless = false;
    let mut minimized = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--headless" => run_headless = true,
            "--minimized" => minimized = true,
            "--config" => {
                let Some(p) = args.get(i + 1) else {
                    eprintln!("--config needs a file name");
                    std::process::exit(2);
                };
                config::set_config_path(p.into());
                i += 1;
            }
            "-V" | "--version" => {
                println!("wled-ambilight {VERSION}");
                return Ok(());
            }
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            // older macOS passes a process serial number when started from Finder
            other if other.starts_with("-psn_") => {}
            other => {
                eprintln!("unknown option: {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    if run_headless {
        headless();
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 880.0])
            .with_min_inner_size([1040.0, 660.0])
            .with_icon(window_icon())
            .with_title("WLED Ambilight")
            .with_app_id(APP_ID),
        ..Default::default()
    };
    eframe::run_native(
        "wled-ambilight",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, minimized)))),
    )
}
