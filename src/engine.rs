use std::collections::HashMap;
use std::time::Instant;

use crate::capture::SharedFrame;
use crate::config::{Config, Protocol};
use crate::sampler::{self, Plan};
use crate::wled;

pub struct Engine {
    pub plan: Plan,
    plan_w: u32,
    plan_h: u32,
    dirty: bool,
    /// smoothed values per device / led
    smoothing: Vec<Vec<[f32; 3]>>,
    /// last three raw samples per device / led, for a median filter
    history: Vec<Vec<[[f32; 3]; 3]>>,
    /// last colours that went out, for the LED preview
    pub last: Vec<Vec<[u8; 3]>>,
    senders: HashMap<String, wled::Sender>,
    sender_keys: HashMap<String, (String, Protocol)>,
    pub sent_frames: u64,
    pub send_fps: f32,
    fps_window: (Instant, u64),
    pub last_sample_us: u128,
    stats_on: bool,
    stat_sum: f64,
    stat_n: u64,
    stat_t: Instant,
    pub flicker: f32,
    /// last time we actually sent to a device (index = device index)
    last_sent: Vec<Instant>,
    stat_led: Vec<f64>,
    stat_led_n: u64,
    /// freeze the picture (own window focused, or user pressed pause)
    pub paused: bool,
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

impl Engine {
    pub fn new(cfg: &Config, w: u32, h: u32) -> Self {
        Self {
            plan: sampler::build(cfg, w.max(1), h.max(1)),
            plan_w: w.max(1),
            plan_h: h.max(1),
            dirty: false,
            smoothing: Vec::new(),
            history: Vec::new(),
            last: Vec::new(),
            senders: HashMap::new(),
            sender_keys: HashMap::new(),
            sent_frames: 0,
            send_fps: 0.0,
            fps_window: (Instant::now(), 0),
            last_sample_us: 0,
            stats_on: std::env::var("AMBILIGHT_STATS").is_ok(),
            stat_sum: 0.0,
            stat_n: 0,
            stat_t: Instant::now(),
            flicker: 0.0,
            last_sent: Vec::new(),
            stat_led: Vec::new(),
            stat_led_n: 0,
            paused: false,
        }
    }





    pub fn tick(&mut self, cfg: &Config, frame: Option<&SharedFrame>, dt: f32) {
        let (w, h) = frame
            .map(|f| (f.w.max(1), f.h.max(1)))
            .unwrap_or((self.plan_w, self.plan_h));
        if self.dirty || w != self.plan_w || h != self.plan_h {
            self.plan = sampler::build(cfg, w, h);
            self.plan_w = w;
            self.plan_h = h;
            self.smoothing.clear();
            self.dirty = false;
        }

        let s_spatial = cfg.settings.spatial;
        let t0 = Instant::now();
        let mut raw: Vec<Vec<[f32; 3]>> = Vec::new();
        if self.paused {
            // keep the current colours on the strip instead of going dark
            {
                for (di, dev) in cfg.devices.iter().enumerate() {
                    if !dev.enabled {
                        continue;
                    }
                    let key = (dev.host.clone(), dev.protocol);
                    let stale = self.sender_keys.get(&dev.id).map(|k| k != &key).unwrap_or(true);
                    if stale {
                        self.senders
                            .insert(dev.id.clone(), wled::Sender::new(&dev.host, dev.protocol));
                        self.sender_keys.insert(dev.id.clone(), key);
                    }
                    if let Some(last) = self.last.get(di).cloned() {
                        if let Some(sender) = self.senders.get_mut(&dev.id) {
                            sender.send(&last);
                        }
                    }
                }
                self.sent_frames += 1;
            }
            return;
        }

        if let Some(f) = frame {
            sampler::sample(&self.plan, &f.data, s_spatial, &mut raw);
        }
        self.last_sample_us = t0.elapsed().as_micros();

        let s = &cfg.settings;
        let tau = (s.smoothing_ms / 1000.0).max(0.0);
        let alpha = if tau <= 0.0 {
            1.0
        } else {
            (1.0 - (-dt.max(0.0) / tau).exp()).clamp(0.0, 1.0)
        };
        if !raw.is_empty() && self.smoothing.len() != raw.len() {
            self.smoothing = raw.clone();
        }


        for (di, dev) in cfg.devices.iter().enumerate() {
            if !dev.enabled {
                continue;
            }
            let Some(plan_dev) = self.plan.devices.get(di) else {
                continue;
            };
            let n = dev.led_count as usize;
            let mut sm = self.smoothing.get(di).cloned().unwrap_or_default();
            if sm.len() != n {
                sm = vec![[0.0, 0.0, 0.0]; n];
            }
            if self.history.len() <= di {
                self.history.resize(di + 1, Vec::new());
            }
            if self.history[di].len() != n {
                self.history[di] = vec![[[0.0; 3]; 3]; n];
            }
            let mut out = vec![[0u8; 3]; n];
            for led in 0..n {
                let mut c = raw
                    .get(di)
                    .and_then(|d| d.get(led))
                    .copied()
                    .unwrap_or([0.0, 0.0, 0.0]);
                c[0] *= plan_dev.gains.get(led).copied().unwrap_or(0.0);
                c[1] *= plan_dev.gains.get(led).copied().unwrap_or(0.0);
                c[2] *= plan_dev.gains.get(led).copied().unwrap_or(0.0);
                if alpha >= 1.0 {
                    sm[led] = c;
                } else {
                    // slew limit: changes per second are capped, so single noisy
                    // frames cannot make an LED jump
                    let max_step = if s.slew > 0.0 {
                        s.slew * dt.max(0.001)
                    } else {
                        f32::INFINITY
                    };
                    for k in 0..3 {
                        let diff = (c[k] - sm[led][k]) * alpha;
                        let step = if s.slew > 0.0 {
                            diff.clamp(-max_step, max_step)
                        } else {
                            diff
                        };
                        if step.abs() > 0.01 || diff.abs() < 0.6 {
                            sm[led][k] += step;
                        }
                    }
                }
                let mut v = sm[led];
                // global grade
                if s.gamma != 1.0 {
                    for k in 0..3 {
                        v[k] = 255.0 * (v[k] / 255.0).clamp(0.0, 1.0).powf(s.gamma);
                    }
                }
                if s.saturation != 1.0 {
                    let (hh, _, vv) = rgb_to_hsv([
                        v[0] / 255.0,
                        v[1] / 255.0,
                        v[2] / 255.0,
                    ]);
                    let g = hsv_to_rgb(hh, s.saturation.clamp(0.0, 2.0), vv);
                    v = [g[0] * 255.0, g[1] * 255.0, g[2] * 255.0];
                }
                let gain = dev.brightness * s.brightness;
                let mut px = [
                    (v[0] * gain).clamp(0.0, 255.0) as u8,
                    (v[1] * gain).clamp(0.0, 255.0) as u8,
                    (v[2] * gain).clamp(0.0, 255.0) as u8,
                ];
                if dev.bgr {
                    px.swap(0, 2);
                }
                out[led] = px;
            }
            if dev.reverse {
                out.reverse();
            }
            if self.last.len() <= di {
                self.last.resize(di + 1, Vec::new());
            }
            if self.stats_on {
                if let Some(prev) = self.last.get(di) {
                    if prev.len() == out.len() {
                        let mut sum = 0.0f64;
                        if self.stat_led.len() != out.len() {
                            self.stat_led = vec![0.0; out.len()];
                        }
                        for (i, (a, b)) in prev.iter().zip(out.iter()).enumerate() {
                            let mut d = 0.0f64;
                            for k in 0..3 {
                                d += (a[k] as f64 - b[k] as f64).abs();
                            }
                            sum += d;
                            self.stat_led[i] += d / 3.0;
                        }
                        self.stat_led_n += 1;
                        self.stat_sum += sum / (out.len().max(1) * 3) as f64;
                        self.stat_n += 1;
                    }
                }
                if self.stat_t.elapsed().as_secs_f32() >= 2.0 {
                    let f = (self.stat_sum / self.stat_n.max(1) as f64) as f32;
                    self.flicker = f;
                    let mut worst: Vec<(usize, f64)> = self
                        .stat_led
                        .iter()
                        .enumerate()
                        .map(|(i, v)| (i, v / self.stat_led_n.max(1) as f64))
                        .collect();
                    worst.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                    worst.truncate(6);
                    let finfo = frame
                        .map(|fr| {
                            let data = &fr.data;
                            let mut sum: u64 = 0;
                            let mut n: u64 = 0;
                            let mut i = 0;
                            while i + 2 < data.len() {
                                sum += (data[i] as u64 + data[i + 1] as u64 + data[i + 2] as u64) / 3;
                                n += 1;
                                i += 997;
                            }
                            format!(
                                "frame {}x{} seq {} mean {:.1}",
                                fr.w,
                                fr.h,
                                fr.seq,
                                sum as f64 / n.max(1) as f64
                            )
                        })
                        .unwrap_or_else(|| "frame NONE".to_string());
                    eprintln!(
                        "dt {:.1} ms  flicker {:.2}/frame  out {:.0} fps  sample {} µs  devices {} raw {}  {}  worst {:?}",
                        dt * 1000.0,
                        f,
                        self.send_fps,
                        self.last_sample_us,
                        self.plan.devices.len(),
                        raw.len(),
                        finfo,
                        worst
                            .iter()
                            .map(|(i, v)| format!("#{}:{:.1}", i, v))
                            .collect::<Vec<_>>()
                    );
                    self.stat_sum = 0.0;
                    self.stat_n = 0;
                    self.stat_led.iter_mut().for_each(|v| *v = 0.0);
                    self.stat_led_n = 0;
                    self.stat_t = Instant::now();
                }
            }
            self.last[di] = out.clone();
            if self.smoothing.len() <= di {
                self.smoothing.resize(di + 1, Vec::new());
            }
            self.smoothing[di] = sm;

            if self.last_sent.len() <= di {
                self.last_sent.resize(di + 1, Instant::now() - std::time::Duration::from_secs(1));
            }
            let interval = if dev.max_fps > 0 {
                1.0 / dev.max_fps as f32
            } else {
                1.0 / s.target_fps.max(1) as f32
            };
            let due = self.last_sent[di].elapsed().as_secs_f32() >= interval - 0.001;
            if due {
                self.last_sent[di] = Instant::now();
                let key = (dev.host.clone(), dev.protocol);
                let stale = self
                    .sender_keys
                    .get(&dev.id)
                    .map(|k| k != &key)
                    .unwrap_or(true);
                if stale {
                    self.senders
                        .insert(dev.id.clone(), wled::Sender::new(&dev.host, dev.protocol));
                    self.sender_keys.insert(dev.id.clone(), key);
                }
                if let Some(sender) = self.senders.get_mut(&dev.id) {
                    sender.send(&out);
                }
                self.sent_frames += 1;
            }
        }

        let (start, count) = self.fps_window;
        let el = start.elapsed().as_secs_f32();
        if el > 1.0 {
            self.send_fps = (self.sent_frames - count) as f32 / el;
            self.fps_window = (Instant::now(), self.sent_frames);
        }
    }
}
