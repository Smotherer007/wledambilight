// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! macOS: ScreenCaptureKit (macOS 13+).
//!
//! The system scales the stream on the GPU to the small size we ask for, so the
//! CPU only converts BGRA -> RGB. Our own app is excluded in the content
//! filter, so the live preview never ends up in the LEDs.
//!
//! The first start triggers the "Screen Recording" permission prompt. After
//! granting it in System Settings the app has to be restarted (macOS rule).

use std::time::Duration;

use screencapturekit::cm::SCFrameStatus;
use screencapturekit::prelude::*;

use super::scale::{self, Order};
use super::{frame_height, CaptureOptions, CaptureStatus, Handles};
use crate::i18n::tr;

fn permission_hint() -> String {
    tr(
        "Keine Berechtigung für Bildschirmaufnahme. Systemeinstellungen → Datenschutz & Sicherheit → Bildschirm- & Systemaudioaufnahme: WLED Ambilight erlauben und die App neu starten.",
        "No screen recording permission. System Settings → Privacy & Security → Screen & System Audio Recording: allow WLED Ambilight, then restart the app.",
    )
    .to_string()
}

pub fn monitors() -> Vec<String> {
    let Ok(content) = SCShareableContent::get() else {
        return Vec::new();
    };
    content
        .displays()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            format!(
                "{} {} · {}×{}",
                tr("Monitor", "Display"),
                i + 1,
                d.width(),
                d.height()
            )
        })
        .collect()
}

pub fn run(opts: CaptureOptions, h: Handles) {
    h.set_status(CaptureStatus::Starting);
    let content = match SCShareableContent::get() {
        Ok(c) => c,
        Err(e) => return h.fail(format!("{} ({e})", permission_hint())),
    };
    let displays = content.displays();
    let Some(display) = displays.get(opts.monitor).or_else(|| displays.first()) else {
        return h.fail(tr("Kein Monitor gefunden", "no display found"));
    };
    let (sw, sh) = (display.width(), display.height());
    h.set_screen(sw, sh);
    let dw = opts.width.min(sw.max(64));
    let dh = frame_height(dw, sw, sh);

    // leave our own windows out of the picture
    let me = std::process::id() as i32;
    let apps = content.applications();
    let own: Vec<&SCRunningApplication> = apps.iter().filter(|a| a.process_id() == me).collect();
    let filter = match SCContentFilter::create()
        .with_display(display)
        .with_excluding_applications(&own, &[])
        .build()
    {
        Ok(f) => f,
        Err(e) => return h.fail(format!("filter: {e}")),
    };

    let fps = opts.fps.clamp(1, 240) as i32;
    let config = SCStreamConfiguration::new()
        .with_width(dw)
        .with_height(dh)
        .with_pixel_format(PixelFormat::BGRA)
        .with_shows_cursor(false)
        .with_queue_depth(3)
        .with_minimum_frame_interval(&CMTime::new(1, fps));

    let mut stream = match SCStream::new(&filter, &config) {
        Ok(s) => s,
        Err(e) => return h.fail(format!("stream: {e}")),
    };

    let out = h.clone();
    let handler = move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
        if !matches!(kind, SCStreamOutputType::Screen) {
            return;
        }
        // idle frames carry no new picture
        if !matches!(sample.frame_status(), Some(SCFrameStatus::Complete) | None) {
            return;
        }
        let Some(pixels) = sample.pixel_buffer() else {
            return;
        };
        let Ok(guard) = pixels.lock_read_only() else {
            return;
        };
        let (w, hgt, stride) = (guard.width(), guard.height(), guard.bytes_per_row());
        // SAFETY: read-only lock held for the whole scope, nothing writes to it.
        let Some(data) = (unsafe { guard.as_slice() }) else {
            return;
        };
        let mut rgb = Vec::new();
        scale::to_rgb(data, w, hgt, stride, Order::Bgra, w, hgt, &mut rgb);
        out.publish(w as u32, hgt as u32, rgb);
        if !matches!(*out.status.lock().unwrap(), CaptureStatus::Running { .. }) {
            out.set_status(CaptureStatus::Running {
                w: w as u32,
                h: hgt as u32,
            });
        }
    };
    if let Err(e) = stream.add_output_handler(handler, SCStreamOutputType::Screen) {
        return h.fail(format!("stream: {e}"));
    }
    if let Err(e) = stream.start_capture() {
        return h.fail(format!("{} ({e})", permission_hint()));
    }

    while !h.stopped() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = stream.stop_capture();
}
