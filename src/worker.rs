// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! The pipeline runs in its own thread: capture -> sample -> WLED.
//!
//! Keeping it out of the GUI has two reasons: the frame rate stays stable no
//! matter what the UI does, and on Linux the window can stay pixel-identical
//! while it is not focused. That matters there because an ambilight captures
//! its own window - if the window keeps redrawing (live preview, fps text, LED
//! bar) it feeds its own output back into the capture and flickers. Windows
//! and macOS exclude our window from the capture instead.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::capture::{CaptureOptions, CaptureStatus, ScreenCapture, SharedFrame};
use crate::config::Config;
use crate::engine::Engine;

/// Everything the UI reads from the pipeline in one snapshot.
#[derive(Clone)]
pub struct Stats {
    pub status: CaptureStatus,
    /// real resolution of the captured screen (e.g. 2560x1440), 0 if unknown
    pub screen: (u32, u32),
    /// size of the capture buffer
    pub dims: (u32, u32),
    pub fps_in: f32,
    pub fps_out: f32,
    pub sample_us: u128,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            status: CaptureStatus::Idle,
            screen: (0, 0),
            dims: (320, 180),
            fps_in: 0.0,
            fps_out: 0.0,
            sample_us: 0,
        }
    }
}

pub struct Shared {
    cfg: Mutex<Config>,
    /// bumped whenever `cfg` changes, so the worker only clones on change
    cfg_version: AtomicU64,
    /// colours that went out on the wire, per device
    pub last: Mutex<Vec<Vec<[u8; 3]>>>,
    /// latest capture frame, for the preview
    pub frame: Mutex<SharedFrame>,
    pub stats: Mutex<Stats>,
    pub paused: Arc<AtomicBool>,
    /// bump to (re)start the screen capture
    restart: AtomicU64,
}

impl Shared {
    pub fn set_config(&self, cfg: &Config) {
        let mut c = self.cfg.lock().unwrap();
        // the portal token is owned by the pipeline; never let the UI wipe it
        let token = c.settings.restore_token.take();
        *c = cfg.clone();
        if c.settings.restore_token.is_none() {
            c.settings.restore_token = token;
        }
        self.cfg_version.fetch_add(1, Ordering::Release);
    }
    /// Forget the portal token, so the next capture start asks again.
    pub fn clear_restore_token(&self) {
        self.cfg.lock().unwrap().settings.restore_token = None;
        self.cfg_version.fetch_add(1, Ordering::Release);
    }
    pub fn restore_token(&self) -> Option<String> {
        self.cfg.lock().unwrap().settings.restore_token.clone()
    }
    pub fn config(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }
    pub fn request_restart(&self) {
        self.restart.fetch_add(1, Ordering::Relaxed);
    }
    pub fn stats(&self) -> Stats {
        self.stats.lock().unwrap().clone()
    }
    /// The capture may hand out a new portal token; store it in the config.
    fn store_token(&self, token: String) {
        let mut c = self.cfg.lock().unwrap();
        if c.settings.restore_token.as_ref() != Some(&token) {
            c.settings.restore_token = Some(token);
            self.cfg_version.fetch_add(1, Ordering::Release);
        }
    }
}

pub struct Pipeline {
    pub shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Pipeline {
    pub fn start(cfg: &Config) -> Self {
        let shared = Arc::new(Shared {
            cfg: Mutex::new(cfg.clone()),
            cfg_version: AtomicU64::new(1),
            last: Mutex::new(Vec::new()),
            frame: Mutex::new(SharedFrame::black(320, 180)),
            stats: Mutex::new(Stats::default()),
            paused: Arc::new(AtomicBool::new(false)),
            restart: AtomicU64::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let sh = shared.clone();
        let sp = stop.clone();
        let join = std::thread::Builder::new()
            .name("pipeline".into())
            .spawn(move || run(sh, sp))
            .ok();
        Self { shared, stop, join }
    }
}

/// Capture settings that need a restart of the capture when they change.
/// (The Linux backend does not limit its rate, so fps changes don't count there.)
fn capture_key(cfg: &Config) -> (u32, usize, u32) {
    let fps = if cfg!(target_os = "linux") {
        0
    } else {
        cfg.settings.target_fps
    };
    (cfg.settings.capture_width, cfg.settings.monitor, fps)
}

fn run(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    let mut capture: Option<ScreenCapture> = None;
    let mut capture_cfg = (0u32, 0usize, 0u32);
    let mut restart_seen = 0u64;
    let mut cfg = shared.config();
    let mut cfg_seen = shared.cfg_version.load(Ordering::Acquire);
    let mut engine = Engine::new(&cfg, 320, 180);
    let mut last = Instant::now();
    let mut next_tick = Instant::now();
    let mut last_seq = 0u64;
    let mut fps_in = (Instant::now(), 0u32);

    while !stop.load(Ordering::Relaxed) {
        let v = shared.cfg_version.load(Ordering::Acquire);
        if v != cfg_seen {
            cfg = shared.config();
            cfg_seen = v;
        }

        // Run on a fine grid that is a multiple of the wanted rate, so per-device
        // caps land on exact values (50 -> 150 Hz, 60 -> 120 Hz, ...).
        let fps = cfg.settings.target_fps.clamp(1, 240) as f32;
        let mult = (120.0 / fps).ceil().max(1.0);
        let tick_hz = (fps * mult).min(240.0);

        let now = Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
            continue;
        }

        // (re)start the screen capture when needed
        let restart = shared.restart.load(Ordering::Relaxed);
        let key = capture_key(&cfg);
        if capture.is_none() || key != capture_cfg || restart != restart_seen {
            restart_seen = restart;
            capture_cfg = key;
            drop(capture.take()); // stop the old one first (releases the portal session)
            capture = Some(ScreenCapture::start(CaptureOptions {
                width: cfg.settings.capture_width,
                fps: cfg.settings.target_fps,
                monitor: cfg.settings.monitor,
                restore_token: cfg.settings.restore_token.clone(),
            }));
        }

        let frame = capture.as_ref().map(|c| c.frame());
        if let Some(c) = capture.as_ref() {
            if let Some(t) = c.restore_token() {
                shared.store_token(t);
            }
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;
        next_tick = now + Duration::from_secs_f32(1.0 / tick_hz);

        engine.paused = shared.paused.load(Ordering::Relaxed);
        engine.tick(&cfg, frame.as_ref(), dt);

        // incoming frame rate
        if let Some(f) = frame.as_ref() {
            if f.seq != last_seq {
                last_seq = f.seq;
                fps_in.1 += 1;
            }
        }
        let el = fps_in.0.elapsed().as_secs_f32();
        let fps_in_now = if el >= 1.0 {
            let v = fps_in.1 as f32 / el;
            fps_in = (Instant::now(), 0);
            Some(v)
        } else {
            None
        };

        if let Some(f) = frame {
            let mut g = shared.frame.lock().unwrap();
            if g.seq != f.seq || g.w != f.w {
                *g = f;
            }
        }
        if let Ok(mut l) = shared.last.lock() {
            l.clone_from(&engine.last);
        }
        if let Ok(mut st) = shared.stats.lock() {
            if let Some(c) = capture.as_ref() {
                st.status = c.status();
                let scr = c.screen();
                if scr.0 > 0 {
                    st.screen = scr;
                }
                if let CaptureStatus::Running { w, h } = st.status {
                    st.dims = (w, h);
                }
            }
            if let Some(v) = fps_in_now {
                st.fps_in = v;
            }
            st.fps_out = engine.send_fps;
            st.sample_us = engine.last_sample_us;
        }
    }
}
