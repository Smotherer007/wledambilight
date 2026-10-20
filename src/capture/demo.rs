// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! `AMBILIGHT_DEMO=1`: an animated test picture instead of the screen.
//! Handy for screenshots, UI work and checking a strip without any capture.

use std::time::{Duration, Instant};

use super::{frame_height, CaptureOptions, CaptureStatus, Handles};

pub fn enabled() -> bool {
    std::env::var_os("AMBILIGHT_DEMO").is_some()
}

pub fn run(opts: CaptureOptions, h: Handles) {
    let (sw, sh) = (2560u32, 1440u32);
    h.set_screen(sw, sh);
    let w = opts.width;
    let fh = frame_height(w, sw, sh);
    h.set_status(CaptureStatus::Running { w, h: fh });
    let start = Instant::now();
    let frame_time = Duration::from_secs_f32(1.0 / opts.fps.clamp(1, 120) as f32);
    while !h.stopped() {
        let t = start.elapsed().as_secs_f32();
        let mut rgb = vec![0u8; (w * fh * 3) as usize];
        for y in 0..fh {
            for x in 0..w {
                let fx = x as f32 / w as f32;
                let fy = y as f32 / fh as f32;
                // slow drifting sunset: warm top, magenta right, teal bottom, violet left
                let r = 0.5 + 0.5 * (6.0 * fx + t * 0.7).sin() * (1.0 - fy * 0.6);
                let g = 0.35 + 0.35 * (5.0 * fy - t * 0.5).sin() * fx;
                let b = 0.5 + 0.5 * (4.0 * (fx + fy) - t * 0.9).cos();
                let vignette = 1.0 - 0.5 * ((fx - 0.5).powi(2) + (fy - 0.5).powi(2));
                let o = ((y * w + x) * 3) as usize;
                rgb[o] = (r * vignette * 255.0).clamp(0.0, 255.0) as u8;
                rgb[o + 1] = (g * vignette * 255.0).clamp(0.0, 255.0) as u8;
                rgb[o + 2] = (b * vignette * 255.0).clamp(0.0, 255.0) as u8;
            }
        }
        h.publish(w, fh, rgb);
        std::thread::sleep(frame_time);
    }
}
