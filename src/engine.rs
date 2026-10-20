// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Colour pipeline: sample -> median -> smoothing -> grading -> send.
//!
//! `tick` runs on a fixed grid (see `worker`). New capture frames are sampled
//! once; ticks in between only advance the smoothing, which keeps fades fluid
//! even when the screen delivers fewer frames than the LEDs get.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::capture::SharedFrame;
use crate::config::{Config, DeviceCfg};
use crate::output::{self, Output};
use crate::sampler::{self, Plan};

pub struct Engine {
    pub plan: Plan,
    plan_w: u32,
    plan_h: u32,
    /// sequence number of the last sampled frame
    sampled_seq: Option<u64>,
    /// latest filtered sample per device / led
    raw: Vec<Vec<[f32; 3]>>,
    /// smoothed values per device / led
    smoothing: Vec<Vec<[f32; 3]>>,
    /// last three raw samples per device / led, for the median filter
    history: Vec<Vec<[[f32; 3]; 3]>>,
    history_pos: usize,
    /// last colours that went out, for the LED preview
    pub last: Vec<Vec<[u8; 3]>>,
    /// one output per device id, with the connection key it was opened for
    outputs: HashMap<String, (String, Box<dyn Output>)>,
    /// frames sent per device (index = device index)
    sent: Vec<u64>,
    /// update rate of the fastest device
    pub send_fps: f32,
    fps_window: (Instant, Vec<u64>),
    pub last_sample_us: u128,
    /// last time we actually sent to a device (index = device index)
    last_sent: Vec<Instant>,
    /// freeze the picture (own window focused, or user pressed pause)
    pub paused: bool,
    stats: Option<Stats>,
}

/// Debug output with `AMBILIGHT_STATS=1`: how much the LEDs change per frame.
struct Stats {
    sum: f64,
    n: u64,
    t: Instant,
}

fn rgb_to_hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0], c[1], c[2]);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, if max <= 0.0 { 0.0 } else { d / max }, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor() as i32;
    let f = h - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

#[inline]
fn median3(a: f32, b: f32, c: f32) -> f32 {
    a.max(b).min(a.min(b).max(c))
}

impl Engine {
    pub fn new(cfg: &Config, w: u32, h: u32) -> Self {
        Self {
            plan: sampler::build(cfg, w.max(1), h.max(1)),
            plan_w: w.max(1),
            plan_h: h.max(1),
            sampled_seq: None,
            raw: Vec::new(),
            smoothing: Vec::new(),
            history: Vec::new(),
            history_pos: 0,
            last: Vec::new(),
            outputs: HashMap::new(),
            sent: Vec::new(),
            send_fps: 0.0,
            fps_window: (Instant::now(), Vec::new()),
            last_sample_us: 0,
            last_sent: Vec::new(),
            paused: false,
            stats: std::env::var("AMBILIGHT_STATS").is_ok().then(|| Stats {
                sum: 0.0,
                n: 0,
                t: Instant::now(),
            }),
        }
    }

    /// Send `colors` to `dev`, (re)opening the output when its connection
    /// settings changed.
    fn send(&mut self, di: usize, dev: &DeviceCfg, colors: &[[u8; 3]]) {
        let key = dev.connection_key();
        let stale = self
            .outputs
            .get(&dev.id)
            .map(|(k, _)| k != &key)
            .unwrap_or(true);
        if stale {
            self.outputs
                .insert(dev.id.clone(), (key, output::open(dev)));
        }
        if let Some((_, out)) = self.outputs.get_mut(&dev.id) {
            out.send(colors);
        }
        if self.sent.len() <= di {
            self.sent.resize(di + 1, 0);
        }
        self.sent[di] += 1;
    }

    /// Is device `di` due for a packet, given its own fps cap?
    fn due(&mut self, di: usize, dev: &DeviceCfg, target_fps: u32) -> bool {
        if self.last_sent.len() <= di {
            self.last_sent
                .resize(di + 1, Instant::now() - Duration::from_secs(1));
        }
        let fps = if dev.max_fps > 0 {
            dev.max_fps
        } else {
            target_fps.max(1)
        };
        let interval = 1.0 / fps as f32;
        if self.last_sent[di].elapsed().as_secs_f32() >= interval - 0.001 {
            self.last_sent[di] = Instant::now();
            true
        } else {
            false
        }
    }

    /// Sample a new frame and run it through the median filter.
    fn sample(&mut self, frame: &SharedFrame, cfg: &Config) {
        let t0 = Instant::now();
        let mut fresh = Vec::new();
        sampler::sample(&self.plan, &frame.data, cfg.settings.spatial, &mut fresh);
        self.last_sample_us = t0.elapsed().as_micros();

        // median of the last three samples per channel removes single-frame
        // spikes (cursor blink, video compression noise) without real lag
        let shape_changed = self.history.len() != fresh.len()
            || self
                .history
                .iter()
                .zip(&fresh)
                .any(|(h, f)| h.len() != f.len());
        if shape_changed {
            self.history = fresh
                .iter()
                .map(|d| d.iter().map(|c| [*c; 3]).collect())
                .collect();
        }
        self.history_pos = (self.history_pos + 1) % 3;
        let use_median = cfg.settings.smoothing_ms > 0.0;
        for (dh, df) in self.history.iter_mut().zip(fresh.iter_mut()) {
            for (h, c) in dh.iter_mut().zip(df.iter_mut()) {
                h[self.history_pos] = *c;
                if use_median {
                    for k in 0..3 {
                        c[k] = median3(h[0][k], h[1][k], h[2][k]);
                    }
                }
            }
        }
        self.raw = fresh;
    }

    /// Connection state of every open output, by device id.
    pub fn statuses(&self) -> HashMap<String, output::Status> {
        self.outputs
            .iter()
            .map(|(id, (_, o))| (id.clone(), o.status()))
            .collect()
    }

    /// Close every output and wait briefly, so sessions end cleanly (Razer
    /// session closed, Govee/Nanoleaf handed back) when the app quits.
    pub fn shutdown(&mut self) {
        for (_, out) in self.outputs.values_mut() {
            out.shutdown(Duration::from_millis(800));
        }
        self.outputs.clear();
    }

    /// Close outputs of devices that were removed or switched off (ends
    /// sessions such as Razer Chroma, gives Govee/Nanoleaf back to their app).
    fn prune(&mut self, cfg: &Config) {
        if self.outputs.len() > cfg.devices.iter().filter(|d| d.enabled).count() {
            self.outputs
                .retain(|id, _| cfg.devices.iter().any(|d| d.enabled && &d.id == id));
        }
    }

    pub fn tick(&mut self, cfg: &Config, frame: Option<&SharedFrame>, dt: f32) {
        self.prune(cfg);
        let (w, h) = frame
            .map(|f| (f.w.max(1), f.h.max(1)))
            .unwrap_or((self.plan_w, self.plan_h));
        // rebuild the cell plan whenever the frame size or the zone layout changed
        if w != self.plan_w || h != self.plan_h || self.plan.layout != sampler::layout_hash(cfg) {
            self.plan = sampler::build(cfg, w, h);
            self.plan_w = w;
            self.plan_h = h;
            self.smoothing.clear();
            self.sampled_seq = None;
        }

        if self.paused {
            // keep the current colours on the strip instead of going dark
            // (WLED would fall back to its own effect after a timeout)
            for (di, dev) in cfg.devices.iter().enumerate() {
                if !dev.enabled || !self.due(di, dev, cfg.settings.target_fps) {
                    continue;
                }
                if let Some(last) = self.last.get(di).cloned() {
                    self.send(di, dev, &last);
                }
            }
            self.update_fps();
            return;
        }

        if let Some(f) = frame {
            if self.sampled_seq != Some(f.seq) {
                self.sampled_seq = Some(f.seq);
                self.sample(f, cfg);
            }
        }

        let s = &cfg.settings;
        let tau = (s.smoothing_ms / 1000.0).max(0.0);
        let alpha = if tau <= 0.0 {
            1.0
        } else {
            (1.0 - (-dt.max(0.0) / tau).exp()).clamp(0.0, 1.0)
        };
        // slew limit: changes per second are capped, so a single noisy frame
        // cannot make an LED jump
        let max_step = if s.slew > 0.0 {
            s.slew * dt.max(0.001)
        } else {
            f32::INFINITY
        };
        let gain_global = s.brightness;

        // enabled devices are planned in order; map config index -> plan index
        let mut plan_idx = 0usize;
        for (di, dev) in cfg.devices.iter().enumerate() {
            if !dev.enabled {
                continue;
            }
            let pi = plan_idx;
            plan_idx += 1;
            let Some(plan_dev) = self.plan.devices.get(pi) else {
                continue;
            };
            let n = dev.led_count as usize;
            if self.smoothing.len() <= di {
                self.smoothing.resize(di + 1, Vec::new());
            }
            let first_fill = self.smoothing[di].len() != n;
            if first_fill {
                self.smoothing[di] = vec![[0.0; 3]; n];
            }
            let raw_dev = self.raw.get(pi);
            let mut out = vec![[0u8; 3]; n];
            for (led, px_out) in out.iter_mut().enumerate() {
                let g = plan_dev.gains.get(led).copied().unwrap_or(0.0);
                let c0 = raw_dev
                    .and_then(|d| d.get(led))
                    .copied()
                    .unwrap_or([0.0; 3]);
                let c = [c0[0] * g, c0[1] * g, c0[2] * g];
                let sm = &mut self.smoothing[di][led];
                if alpha >= 1.0 || first_fill {
                    *sm = c;
                } else {
                    for k in 0..3 {
                        let diff = (c[k] - sm[k]) * alpha;
                        let step = diff.clamp(-max_step, max_step);
                        // dead band: tiny steps far from the target are noise
                        if step.abs() > 0.01 || diff.abs() < 0.6 {
                            sm[k] += step;
                        }
                    }
                }
                let mut v = *sm;
                if s.gamma != 1.0 {
                    for x in v.iter_mut() {
                        *x = 255.0 * (*x / 255.0).clamp(0.0, 1.0).powf(s.gamma);
                    }
                }
                if s.saturation != 1.0 {
                    let (hh, ss, vv) = rgb_to_hsv([v[0] / 255.0, v[1] / 255.0, v[2] / 255.0]);
                    let g = hsv_to_rgb(hh, (ss * s.saturation).clamp(0.0, 1.0), vv);
                    v = [g[0] * 255.0, g[1] * 255.0, g[2] * 255.0];
                }
                let gain = dev.brightness * gain_global;
                let mut px = [
                    (v[0] * gain).clamp(0.0, 255.0) as u8,
                    (v[1] * gain).clamp(0.0, 255.0) as u8,
                    (v[2] * gain).clamp(0.0, 255.0) as u8,
                ];
                if dev.bgr {
                    px.swap(0, 2);
                }
                *px_out = px;
            }
            if dev.reverse {
                out.reverse();
            }
            if self.last.len() <= di {
                self.last.resize(di + 1, Vec::new());
            }
            if let Some(st) = self.stats.as_mut() {
                if let Some(prev) = self.last.get(di).filter(|p| p.len() == out.len()) {
                    let d: f64 = prev
                        .iter()
                        .zip(&out)
                        .map(|(a, b)| {
                            (0..3)
                                .map(|k| (a[k] as f64 - b[k] as f64).abs())
                                .sum::<f64>()
                        })
                        .sum();
                    st.sum += d / (out.len().max(1) * 3) as f64;
                    st.n += 1;
                }
            }
            if self.due(di, dev, s.target_fps) {
                self.send(di, dev, &out);
            }
            self.last[di] = out;
        }

        if let Some(st) = self.stats.as_mut() {
            if st.t.elapsed().as_secs_f32() >= 2.0 {
                eprintln!(
                    "dt {:.1} ms  change {:.2}/frame  out {:.0} fps  sample {} µs  devices {}",
                    dt * 1000.0,
                    st.sum / st.n.max(1) as f64,
                    self.send_fps,
                    self.last_sample_us,
                    self.plan.devices.len(),
                );
                st.sum = 0.0;
                st.n = 0;
                st.t = Instant::now();
            }
        }
        self.update_fps();
    }

    fn update_fps(&mut self) {
        let el = self.fps_window.0.elapsed().as_secs_f32();
        if el > 1.0 {
            let prev = &self.fps_window.1;
            let best = self
                .sent
                .iter()
                .enumerate()
                .map(|(i, n)| n - prev.get(i).copied().unwrap_or(0).min(*n))
                .max()
                .unwrap_or(0);
            self.send_fps = best as f32 / el;
            self.fps_window = (Instant::now(), self.sent.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DeviceCfg, Zone, ZonePreset};
    use std::sync::Arc;

    fn setup(leds: u16) -> Config {
        let mut cfg = Config::default();
        // TEST-NET address, nothing listens there
        cfg.devices.push(DeviceCfg::new("192.0.2.1", "t", leds));
        cfg.zones.push(Zone::preset(
            ZonePreset::Full,
            "192.0.2.1".into(),
            leds,
            [0, 0, 0],
        ));
        cfg.settings.smoothing_ms = 0.0;
        cfg
    }

    fn solid(w: u32, h: u32, rgb: [u8; 3], seq: u64) -> SharedFrame {
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..w * h {
            data.extend_from_slice(&rgb);
        }
        SharedFrame {
            w,
            h,
            data: Arc::new(data),
            seq,
        }
    }

    #[test]
    fn passes_colour_through_without_smoothing() {
        let cfg = setup(8);
        let mut e = Engine::new(&cfg, 32, 18);
        e.tick(&cfg, Some(&solid(32, 18, [200, 100, 50], 1)), 0.02);
        assert_eq!(e.last[0][0], [200, 100, 50]);
        assert_eq!(e.last[0].len(), 8);
    }

    #[test]
    fn median_rejects_a_single_spike() {
        let mut cfg = setup(4);
        cfg.settings.smoothing_ms = 1.0; // enables the median, nearly no lag
        cfg.settings.slew = 0.0;
        let mut e = Engine::new(&cfg, 32, 18);
        for seq in 1..=3 {
            e.tick(&cfg, Some(&solid(32, 18, [10, 10, 10], seq)), 1.0);
        }
        e.tick(&cfg, Some(&solid(32, 18, [250, 250, 250], 4)), 1.0);
        assert!(e.last[0][0][0] < 20, "spike leaked: {:?}", e.last[0][0]);
    }

    #[test]
    fn bgr_and_reverse() {
        let mut cfg = setup(2);
        cfg.devices[0].bgr = true;
        let mut e = Engine::new(&cfg, 32, 18);
        e.tick(&cfg, Some(&solid(32, 18, [1, 2, 3], 1)), 0.02);
        assert_eq!(e.last[0][0], [3, 2, 1]);
    }

    #[test]
    fn saturation_zero_is_grey() {
        let mut cfg = setup(1);
        cfg.settings.saturation = 0.0;
        let mut e = Engine::new(&cfg, 32, 18);
        e.tick(&cfg, Some(&solid(32, 18, [200, 0, 0], 1)), 0.02);
        let c = e.last[0][0];
        assert_eq!(c[0], c[1]);
        assert_eq!(c[1], c[2]);
    }

    #[test]
    fn median3_works() {
        assert_eq!(median3(1.0, 5.0, 3.0), 3.0);
        assert_eq!(median3(9.0, 1.0, 1.0), 1.0);
    }
}
