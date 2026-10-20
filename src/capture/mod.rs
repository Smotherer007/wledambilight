// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Screen capture, one backend per operating system.
//!
//! | OS      | Backend                                              |
//! |---------|------------------------------------------------------|
//! | Linux   | xdg-desktop-portal ScreenCast + PipeWire (GStreamer) |
//! | Windows | Windows Graphics Capture                             |
//! | macOS   | ScreenCaptureKit                                     |
//!
//! Every backend delivers small RGB frames (`capture_width` wide, height follows
//! the screen aspect) into a shared slot. The rest of the program never sees
//! the native resolution, so zones work the same everywhere.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

mod demo;
pub mod scale;

#[cfg(target_os = "windows")]
pub use windows::exclude_window_from_capture;

/// True when the backend leaves our own window out of the picture. Then the
/// preview can stay live without feeding the app's own pixels back into the
/// LEDs. On Linux the portal cannot do that, so the UI freezes its preview
/// while unfocused instead.
pub const EXCLUDES_OWN_WINDOW: bool = cfg!(any(target_os = "windows", target_os = "macos"));

/// Whether the user can pick a monitor in the app (on Linux the portal asks).
pub const HAS_MONITOR_CHOICE: bool = cfg!(any(target_os = "windows", target_os = "macos"));

#[derive(Debug, Clone, PartialEq)]
pub enum CaptureStatus {
    Idle,
    Starting,
    Running { w: u32, h: u32 },
    Error(String),
}

/// One captured frame, tightly packed RGB. The pixel data is reference counted
/// so handing a frame to the pipeline and the preview is free.
#[derive(Clone)]
pub struct SharedFrame {
    pub w: u32,
    pub h: u32,
    pub data: Arc<Vec<u8>>,
    pub seq: u64,
}

impl SharedFrame {
    pub fn black(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            data: Arc::new(vec![0; (w * h * 3) as usize]),
            seq: 0,
        }
    }
}

pub type FrameSlot = Arc<Mutex<SharedFrame>>;

#[derive(Debug, Clone)]
pub struct CaptureOptions {
    /// width of the delivered frames
    pub width: u32,
    /// upper bound for the frame rate the backend should deliver
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub fps: u32,
    /// Windows/macOS: monitor index
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub monitor: usize,
    /// Linux: portal restore token
    pub restore_token: Option<String>,
}

/// Everything a backend writes to. Cheap to clone, shared with the owner.
#[derive(Clone)]
pub struct Handles {
    pub slot: FrameSlot,
    pub screen: Arc<Mutex<(u32, u32)>>,
    pub status: Arc<Mutex<CaptureStatus>>,
    pub restore_token: Arc<Mutex<Option<String>>>,
    pub stop: Arc<AtomicBool>,
}

impl Handles {
    pub fn set_status(&self, s: CaptureStatus) {
        *self.status.lock().unwrap() = s;
    }

    pub fn fail(&self, msg: impl Into<String>) {
        self.set_status(CaptureStatus::Error(msg.into()));
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn set_screen(&self, w: u32, h: u32) {
        if w > 0 && h > 0 {
            *self.screen.lock().unwrap() = (w, h);
        }
    }

    /// Hand a finished RGB frame to the pipeline.
    pub fn publish(&self, w: u32, h: u32, rgb: Vec<u8>) {
        debug_assert_eq!(rgb.len(), (w * h * 3) as usize);
        let mut g = self.slot.lock().unwrap();
        g.w = w;
        g.h = h;
        g.data = Arc::new(rgb);
        g.seq = g.seq.wrapping_add(1);
    }
}

/// Height of the capture buffer for a screen of `sw × sh` at `width`.
pub fn frame_height(width: u32, sw: u32, sh: u32) -> u32 {
    if sw > 0 && sh > 0 {
        ((width as f32 * sh as f32 / sw as f32).round() as u32).clamp(32, 2160) & !1
    } else {
        width * 9 / 16
    }
}

pub struct ScreenCapture {
    handles: Handles,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        self.handles.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl ScreenCapture {
    /// Spawn the capture thread for the current platform.
    pub fn start(mut opts: CaptureOptions) -> Self {
        opts.width = opts.width.clamp(64, 3840);
        let handles = Handles {
            slot: Arc::new(Mutex::new(SharedFrame::black(
                opts.width,
                opts.width * 9 / 16,
            ))),
            screen: Arc::new(Mutex::new((0, 0))),
            status: Arc::new(Mutex::new(CaptureStatus::Starting)),
            restore_token: Arc::new(Mutex::new(opts.restore_token.clone())),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let h = handles.clone();
        let join = std::thread::Builder::new()
            .name("capture".into())
            .spawn(move || {
                if demo::enabled() {
                    demo::run(opts, h)
                } else {
                    platform::run(opts, h)
                }
            })
            .ok();
        if join.is_none() {
            handles.fail("could not start the capture thread");
        }
        Self { handles, join }
    }

    pub fn status(&self) -> CaptureStatus {
        self.handles.status.lock().unwrap().clone()
    }

    pub fn frame(&self) -> SharedFrame {
        self.handles.slot.lock().unwrap().clone()
    }

    pub fn screen(&self) -> (u32, u32) {
        *self.handles.screen.lock().unwrap()
    }

    pub fn restore_token(&self) -> Option<String> {
        self.handles.restore_token.lock().unwrap().clone()
    }
}

/// True in demo mode (`AMBILIGHT_DEMO=1`): nothing is captured, so the preview
/// can never feed back into the LEDs.
pub fn demo_mode() -> bool {
    demo::enabled()
}

/// Monitors the user can choose from (empty on Linux, where the portal asks).
pub fn monitors() -> Vec<String> {
    platform::monitors()
}
