// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Tray / menu bar icon: show window, pause, quit.
//!
//! - Linux: StatusNotifierItem via `ksni` (KDE, GNOME with AppIndicator, Xfce, …)
//! - Windows / macOS: native notification area / menu bar via `tray-icon`

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Tray;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod native;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use native::Tray;

/// Raw RGBA tray icons, rendered from `assets/icon.svg`.
pub const ICON_32: &[u8] = include_bytes!("../../assets/icons/tray-32.rgba");
pub const ICON_64: &[u8] = include_bytes!("../../assets/icons/tray-64.rgba");

#[derive(Clone)]
pub struct TrayState {
    /// user asked for pause (independent of the focus based pause)
    pub manual_pause: Arc<AtomicBool>,
    /// the flag the pipeline actually reads
    pub effective_pause: Arc<AtomicBool>,
    pub show: Arc<AtomicBool>,
    pub quit: Arc<AtomicBool>,
    pub ctx: Arc<Mutex<Option<egui::Context>>>,
    pub status: Arc<Mutex<String>>,
}

impl TrayState {
    pub fn new(effective_pause: Arc<AtomicBool>, ctx: egui::Context) -> Self {
        Self {
            manual_pause: Arc::new(AtomicBool::new(false)),
            effective_pause,
            show: Arc::new(AtomicBool::new(false)),
            quit: Arc::new(AtomicBool::new(false)),
            ctx: Arc::new(Mutex::new(Some(ctx))),
            status: Arc::new(Mutex::new(String::new())),
        }
    }

    fn wake(&self) {
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }

    pub fn request_show(&self) {
        self.show.store(true, Ordering::Relaxed);
        self.wake();
    }

    pub fn toggle_pause(&self) {
        let now = !self.manual_pause.load(Ordering::Relaxed);
        self.manual_pause.store(now, Ordering::Relaxed);
        // while the window is hidden no ui update happens, so set the flag here
        self.effective_pause.store(now, Ordering::Relaxed);
        self.wake();
    }

    pub fn quit(&self) {
        self.quit.store(true, Ordering::Relaxed);
        self.wake();
        // the event loop may not run a pass while minimised - make sure it ends
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(1200));
            std::process::exit(0);
        });
    }
}
