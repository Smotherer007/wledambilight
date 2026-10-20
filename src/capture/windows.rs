// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Windows: Windows Graphics Capture (Windows 10 1903+).
//!
//! WGC hands us full resolution BGRA/RGBA textures. We drop frames above the
//! wanted rate *before* reading them back from the GPU, then box-downscale the
//! rest into the small RGB buffer.
//!
//! Our own window is hidden from every capture API with
//! `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`, so the live preview can
//! never leak into the LEDs.

use std::time::{Duration, Instant};

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use super::scale::{self, Order};
use super::{frame_height, CaptureOptions, CaptureStatus, Handles};
use crate::i18n::tr;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

struct Flags {
    handles: Handles,
    width: u32,
    fps: u32,
}

struct Handler {
    h: Handles,
    width: u32,
    interval: Duration,
    last: Option<Instant>,
    scratch: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = BoxError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let f = ctx.flags;
        Ok(Self {
            h: f.handles,
            width: f.width,
            interval: Duration::from_secs_f32(1.0 / f.fps.clamp(1, 240) as f32),
            last: None,
            scratch: Vec::new(),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.h.stopped() {
            control.stop();
            return Ok(());
        }
        let now = Instant::now();
        if let Some(t) = self.last {
            // small tolerance so 60 Hz displays still deliver 60 fps at a 60 fps target
            if now.duration_since(t) + Duration::from_micros(1500) < self.interval {
                return Ok(());
            }
        }
        self.last = Some(now);

        let mut buf = frame.buffer()?;
        let (sw, sh) = (buf.width(), buf.height());
        let stride = buf.row_pitch() as usize;
        let order = match buf.color_format() {
            ColorFormat::Bgra8 => Order::Bgra,
            _ => Order::Rgba,
        };
        self.h.set_screen(sw, sh);
        let dw = self.width.min(sw.max(1));
        let dh = frame_height(dw, sw, sh).min(sh.max(1));
        let raw = buf.as_raw_buffer();
        scale::to_rgb(
            raw,
            sw as usize,
            sh as usize,
            stride,
            order,
            dw as usize,
            dh as usize,
            &mut self.scratch,
        );
        let rgb = std::mem::take(&mut self.scratch);
        self.h.publish(dw, dh, rgb);
        if !matches!(
            *self.h.status.lock().unwrap(),
            CaptureStatus::Running { .. }
        ) {
            self.h.set_status(CaptureStatus::Running { w: dw, h: dh });
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        if !self.h.stopped() {
            self.h.fail(tr(
                "Monitor nicht mehr verfügbar – Aufnahme neu starten",
                "monitor went away – restart the capture",
            ));
        }
        Ok(())
    }
}

/// All monitors, the primary one first (index 0 = default).
fn ordered_monitors() -> Vec<Monitor> {
    let mut list = Monitor::enumerate().unwrap_or_default();
    if let Ok(primary) = Monitor::primary() {
        let p = primary.as_raw_hmonitor();
        if let Some(i) = list.iter().position(|m| m.as_raw_hmonitor() == p) {
            let m = list.remove(i);
            list.insert(0, m);
        }
    }
    list
}

fn pick_monitor(index: usize) -> Result<Monitor, String> {
    if let Some(m) = ordered_monitors().into_iter().nth(index) {
        return Ok(m);
    }
    Monitor::primary().map_err(|e| format!("monitor: {e}"))
}

fn settings(monitor: Monitor, flags: Flags, tidy: bool) -> Settings<Flags, Monitor> {
    // `tidy` asks for no cursor and no yellow border. Older Windows 10 builds
    // reject those switches, so we retry with defaults.
    let (cursor, border) = if tidy {
        (
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
        )
    } else {
        (CursorCaptureSettings::Default, DrawBorderSettings::Default)
    };
    Settings::new(
        monitor,
        cursor,
        border,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    )
}

pub fn monitors() -> Vec<String> {
    ordered_monitors()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let name = m
                .name()
                .ok()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| format!("{} {}", tr("Monitor", "Display"), i + 1));
            let size = match (m.width(), m.height()) {
                (Ok(w), Ok(h)) => format!(" · {w}×{h}"),
                _ => String::new(),
            };
            let primary = if i == 0 {
                tr(" (Haupt)", " (main)")
            } else {
                ""
            };
            format!("{name}{size}{primary}")
        })
        .collect()
}

pub fn run(opts: CaptureOptions, h: Handles) {
    h.set_status(CaptureStatus::Starting);
    let mk_flags = || Flags {
        handles: h.clone(),
        width: opts.width,
        fps: opts.fps,
    };
    let control = match pick_monitor(opts.monitor).and_then(|m| {
        Handler::start_free_threaded(settings(m, mk_flags(), true)).map_err(|e| e.to_string())
    }) {
        Ok(c) => c,
        Err(_) => match pick_monitor(opts.monitor).and_then(|m| {
            Handler::start_free_threaded(settings(m, mk_flags(), false)).map_err(|e| e.to_string())
        }) {
            Ok(c) => c,
            Err(e) => {
                return h.fail(format!(
                    "{}: {e}",
                    tr(
                        "Bildschirmaufnahme nicht möglich",
                        "screen capture unavailable"
                    )
                ))
            }
        },
    };
    while !h.stopped() && !control.is_finished() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = control.stop();
}

/// Hide a window from screen capture (Windows 10 2004+). Older systems ignore it.
pub fn exclude_window_from_capture(handle: raw_window_handle::RawWindowHandle) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    };
    if let raw_window_handle::RawWindowHandle::Win32(w) = handle {
        unsafe {
            SetWindowDisplayAffinity(w.hwnd.get() as _, WDA_EXCLUDEFROMCAPTURE);
        }
    }
}
