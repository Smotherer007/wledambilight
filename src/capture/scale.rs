// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Fast box downscaling of 32-bit frames (BGRA / RGBA) into packed RGB.
//!
//! The Windows backend receives full resolution frames; the macOS backend gets
//! frames that are already scaled but padded. Both end up here. Every output
//! pixel averages a small grid of samples spread over its source box, which is
//! as stable as a full average for ambilight purposes at a fraction of the cost.

// Linux gets its frames already scaled by GStreamer; the code stays compiled
// there so the unit tests run on every platform.
#![cfg_attr(target_os = "linux", allow(dead_code))]

/// Channel order of the 4-byte source pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    // macOS always asks ScreenCaptureKit for BGRA; Windows and the tests use both
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Rgba,
    Bgra,
}

/// Samples per axis and output pixel.
const GRID: usize = 4;

/// Downscale `src` (`sw × sh` pixels, `stride` bytes per row) to `dw × dh` RGB.
/// `out` is resized as needed and fully overwritten.
#[allow(clippy::too_many_arguments)]
pub fn to_rgb(
    src: &[u8],
    sw: usize,
    sh: usize,
    stride: usize,
    order: Order,
    dw: usize,
    dh: usize,
    out: &mut Vec<u8>,
) {
    out.clear();
    out.resize(dw * dh * 3, 0);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || stride < sw * 4 {
        return;
    }
    let (ri, bi) = match order {
        Order::Rgba => (0, 2),
        Order::Bgra => (2, 0),
    };
    // column sample positions are the same for every row - compute once
    // (first sample x, step, samples)
    let mut cols: Vec<(usize, usize, usize)> = Vec::with_capacity(dw);
    for dx in 0..dw {
        let x0 = dx * sw / dw;
        let x1 = ((dx + 1) * sw / dw).max(x0 + 1).min(sw);
        let n = (x1 - x0).min(GRID);
        let step = ((x1 - x0) / n).max(1);
        cols.push((x0 + step / 2, step, n));
    }
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = ((dy + 1) * sh / dh).max(y0 + 1).min(sh);
        let ny = (y1 - y0).min(GRID);
        let ystep = ((y1 - y0) / ny).max(1);
        let row_out = dy * dw * 3;
        for (dx, &(xs, xstep, nx)) in cols.iter().enumerate() {
            let mut sum = [0u32; 3];
            let mut n = 0u32;
            let mut y = y0 + ystep / 2;
            for _ in 0..ny {
                if y >= sh {
                    break;
                }
                let row = y * stride;
                let mut x = xs;
                for _ in 0..nx {
                    if x >= sw {
                        break;
                    }
                    let o = row + x * 4;
                    if let Some(p) = src.get(o..o + 4) {
                        sum[0] += p[ri] as u32;
                        sum[1] += p[1] as u32;
                        sum[2] += p[bi] as u32;
                        n += 1;
                    }
                    x += xstep;
                }
                y += ystep;
            }
            if let Some(o) = out.get_mut(row_out + dx * 3..row_out + dx * 3 + 3) {
                for k in 0..3 {
                    o[k] = sum[k].checked_div(n).unwrap_or(0) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize, pad: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        let stride = w * 4 + pad;
        let mut v = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                v[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&f(x, y));
            }
        }
        v
    }

    #[test]
    fn halves_keep_their_colour() {
        // left half red, right half blue, BGRA with row padding
        let src = frame(200, 100, 32, |x, _| {
            if x < 100 {
                [0, 0, 255, 255]
            } else {
                [255, 0, 0, 255]
            }
        });
        let mut out = Vec::new();
        to_rgb(&src, 200, 100, 200 * 4 + 32, Order::Bgra, 4, 2, &mut out);
        assert_eq!(&out[0..3], &[255, 0, 0]);
        assert_eq!(&out[9..12], &[0, 0, 255]);
    }

    #[test]
    fn rgba_and_identity_size() {
        let src = frame(8, 4, 0, |x, y| [x as u8 * 10, y as u8 * 10, 7, 255]);
        let mut out = Vec::new();
        to_rgb(&src, 8, 4, 32, Order::Rgba, 8, 4, &mut out);
        // pixel x=5, y=3
        let i = (3 * 8 + 5) * 3;
        assert_eq!(&out[i..i + 3], &[50, 30, 7]);
    }

    #[test]
    fn upscale_and_bad_input_do_not_panic() {
        let src = frame(3, 3, 0, |_, _| [9, 9, 9, 255]);
        let mut out = Vec::new();
        to_rgb(&src, 3, 3, 12, Order::Rgba, 10, 7, &mut out);
        assert_eq!(out.len(), 10 * 7 * 3);
        assert!(out.iter().all(|v| *v == 9));
        to_rgb(&src, 3, 3, 4, Order::Rgba, 10, 7, &mut out); // stride too small
        assert!(out.iter().all(|v| *v == 0));
    }
}
