//! The pipeline runs in its own thread: capture -> sample -> WLED.
//!
//! Keeping it out of the GUI has two reasons: the frame rate stays stable no
//! matter what the UI does, and the window can stay pixel-identical while it is
//! not focused. That last part matters because an ambilight captures its own
//! window - if the window keeps redrawing (live preview, fps text, LED bar) it
//! feeds its own output back into the capture and flickers.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::capture::{CaptureStatus, ScreenCapture, SharedFrame};
use crate::config::Config;
use crate::engine::Engine;

pub struct Shared {
    pub cfg: Mutex<Config>,
    /// colours that went out on the wire, per device
    pub last: Mutex<Vec<Vec<[u8; 3]>>>,
    /// latest capture frame, for the preview
    pub frame: Mutex<SharedFrame>,
    pub dims: Mutex<(u32, u32)>,
    /// real resolution of the captured screen (e.g. 2560x1440)
    pub screen: Mutex<(u32, u32)>,
    pub status: Mutex<CaptureStatus>,
    pub fps_out: Mutex<f32>,
    pub fps_in: Mutex<f32>,
    pub sample_us: Mutex<u128>,
    pub paused: Arc<AtomicBool>,
    /// bump to (re)start the screen capture
    pub restart: AtomicU64,
    restart_seen: AtomicU64,
}

impl Shared {
    pub fn set_config(&self, cfg: &Config) {
        *self.cfg.lock().unwrap() = cfg.clone();
    }
    pub fn config(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }
    pub fn request_restart(&self) {
        self.restart.fetch_add(1, Ordering::Relaxed);
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
            last: Mutex::new(Vec::new()),
            frame: Mutex::new(SharedFrame::black(320, 180)),
            dims: Mutex::new((320, 180)),
            screen: Mutex::new((0, 0)),
            status: Mutex::new(CaptureStatus::Idle),
            fps_out: Mutex::new(0.0),
            fps_in: Mutex::new(0.0),
            sample_us: Mutex::new(0),
            paused: Arc::new(AtomicBool::new(false)),
            restart: AtomicU64::new(0),
            restart_seen: AtomicU64::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let sh = shared.clone();
        let sp = stop.clone();
        let join = std::thread::spawn(move || run(sh, sp));
        Self {
            shared,
            stop,
            join: Some(join),
        }
    }
}

fn run(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    let mut capture: Option<ScreenCapture> = None;
    let mut capture_width = 0u32;
    let mut engine = Engine::new(&shared.config(), 320, 180);
    let mut last = Instant::now();
    let mut next_tick = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let cfg = shared.config();

        // Run the pipeline on a fine grid that is a multiple of the wanted rate,
        // so per-device caps land on exact values (50 -> 150 Hz, 60 -> 120 Hz, ...).
        let fps = cfg.settings.target_fps.clamp(1, 240) as f32;
        let mult = (120.0 / fps).ceil().max(1.0);
        let tick_hz = (fps * mult).min(240.0);
        let target = 1.0 / tick_hz;

        let now = Instant::now();
        if now < next_tick {
            // sleep exactly until the next tick instead of polling
            std::thread::sleep(next_tick - now);
            continue;
        }

        // (re)start the screen capture when needed
        let width = cfg.settings.capture_width;
        let restart =
            shared.restart.load(Ordering::Relaxed) != shared.restart_seen.load(Ordering::Relaxed);
        if restart {
            shared
                .restart_seen
                .store(shared.restart.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        if capture.is_none() || width != capture_width || restart {
            capture = Some(ScreenCapture::start(width, cfg.settings.restore_token.clone()));
            capture_width = width;
        }
        if let Some(c) = capture.as_ref() {
            let st = c.status();
            if let Ok(t) = c.restore_token.lock() {
                if let Some(t) = t.clone() {
                    if cfg.settings.restore_token.as_ref() != Some(&t) {
                        let mut c2 = shared.cfg.lock().unwrap();
                        c2.settings.restore_token = Some(t);
                    }
                }
            }
            *shared.status.lock().unwrap() = st;
            if let Ok(sz) = c.screen.lock() {
                if sz.0 > 0 {
                    *shared.screen.lock().unwrap() = *sz;
                }
            }
        } else {
            *shared.status.lock().unwrap() = CaptureStatus::Idle;
        }

        // frame of this tick
        let frame: Option<SharedFrame> = capture.as_ref().map(|c| c.slot.lock().unwrap().clone());

        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;
        next_tick = now + Duration::from_secs_f32(target);

        engine.paused = shared.paused.load(Ordering::Relaxed);
        engine.tick(&cfg, frame.as_ref(), dt);

        if let Some(f) = frame {
            *shared.dims.lock().unwrap() = (f.w, f.h);
            let mut g = shared.frame.lock().unwrap();
            g.w = f.w;
            g.h = f.h;
            g.data = f.data;
            g.seq = f.seq;
        }
        if let Ok(mut l) = shared.last.lock() {
            *l = engine.last.clone();
        }
        *shared.fps_out.lock().unwrap() = engine.send_fps;
        *shared.sample_us.lock().unwrap() = engine.last_sample_us;
    }
}
