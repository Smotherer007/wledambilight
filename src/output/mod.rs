// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Device outputs: one per configured device.
//!
//! WLED is plain UDP and is sent inline from the pipeline thread. Every other
//! kind talks TCP/HTTP or needs a handshake, so it runs in its own thread
//! ([`Threaded`]): the pipeline only drops the newest frame into a slot and
//! never waits for a slow or unreachable device. Lost connections are
//! re-established automatically with a back-off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::config::{DeviceCfg, DeviceKind};

pub mod govee;
pub mod nanoleaf;
pub mod openrgb;
pub mod razer;

/// Connection state shown on the device card.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Status {
    #[default]
    Connecting,
    Ok,
    Error(String),
}

pub trait Output: Send {
    /// Hand over the colours of one frame (wire order, one entry per LED).
    fn send(&mut self, colors: &[[u8; 3]]);
    fn status(&self) -> Status {
        Status::Ok
    }
    /// Close the connection and wait up to `wait` for it (app exit).
    fn shutdown(&mut self, _wait: Duration) {}
}

/// Open the output for a device.
pub fn open(dev: &DeviceCfg) -> Box<dyn Output> {
    match dev.kind {
        DeviceKind::Wled => Box::new(WledOutput(crate::wled::Sender::new(
            &dev.host,
            dev.protocol,
        ))),
        DeviceKind::OpenRgb => Box::new(Threaded::spawn(openrgb::Driver::new(dev))),
        DeviceKind::Razer => Box::new(Threaded::spawn(razer::Driver::new(dev))),
        DeviceKind::Govee => Box::new(Threaded::spawn(govee::Driver::new(dev))),
        DeviceKind::Nanoleaf => Box::new(Threaded::spawn(nanoleaf::Driver::new(dev))),
    }
}

struct WledOutput(crate::wled::Sender);

impl Output for WledOutput {
    fn send(&mut self, colors: &[[u8; 3]]) {
        self.0.send(colors);
    }
}

// ------------------------------------------------------------------ threaded

/// A device protocol that runs in its own thread.
pub trait Driver: Send + 'static {
    /// Open the connection / session. Called again after every error.
    fn connect(&mut self) -> Result<(), String>;
    /// Send one frame.
    fn write(&mut self, colors: &[[u8; 3]]) -> Result<(), String>;
    /// Called about once per second while connected (heartbeats).
    fn keepalive(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// Leave the device in a sane state (end the session, give control back).
    fn close(&mut self) {}
}

struct Slot {
    frame: Option<Vec<[u8; 3]>>,
    stop: bool,
}

pub struct Threaded {
    slot: Arc<(Mutex<Slot>, Condvar)>,
    status: Arc<Mutex<Status>>,
    stopped: Arc<AtomicBool>,
}

impl Threaded {
    pub fn spawn<D: Driver>(mut driver: D) -> Self {
        let slot = Arc::new((
            Mutex::new(Slot {
                frame: None,
                stop: false,
            }),
            Condvar::new(),
        ));
        let status = Arc::new(Mutex::new(Status::Connecting));
        let stopped = Arc::new(AtomicBool::new(false));
        let (s, st, done) = (slot.clone(), status.clone(), stopped.clone());
        let _ = std::thread::Builder::new()
            .name("output".into())
            .spawn(move || {
                run(&mut driver, &s, &st);
                done.store(true, Ordering::Relaxed);
            });
        Self {
            slot,
            status,
            stopped,
        }
    }
}

fn run<D: Driver>(driver: &mut D, slot: &(Mutex<Slot>, Condvar), status: &Mutex<Status>) {
    let set = |s: Status| *status.lock().unwrap() = s;
    let mut connected = false;
    let mut backoff = Duration::from_millis(500);
    let mut last_keepalive = Instant::now();
    loop {
        if !connected {
            if slot.0.lock().unwrap().stop {
                break;
            }
            match driver.connect() {
                Ok(()) => {
                    connected = true;
                    backoff = Duration::from_millis(500);
                    last_keepalive = Instant::now();
                    set(Status::Ok);
                }
                Err(e) => {
                    set(Status::Error(e));
                    // wait, but wake up early when asked to stop
                    let g = slot.0.lock().unwrap();
                    let (g, _) = slot.1.wait_timeout_while(g, backoff, |s| !s.stop).unwrap();
                    if g.stop {
                        break;
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(10));
                    continue;
                }
            }
        }
        // wait for the next frame (or a second for the keep-alive)
        let frame = {
            let g = slot.0.lock().unwrap();
            let (mut g, _) = slot
                .1
                .wait_timeout_while(g, Duration::from_secs(1), |s| s.frame.is_none() && !s.stop)
                .unwrap();
            if g.stop {
                break;
            }
            g.frame.take()
        };
        let result = match frame {
            Some(colors) => driver.write(&colors),
            None => Ok(()),
        };
        let result = result.and_then(|_| {
            if last_keepalive.elapsed() >= Duration::from_secs(1) {
                last_keepalive = Instant::now();
                driver.keepalive()
            } else {
                Ok(())
            }
        });
        if let Err(e) = result {
            connected = false;
            set(Status::Error(e));
        }
    }
    if connected {
        driver.close();
    }
}

impl Output for Threaded {
    fn send(&mut self, colors: &[[u8; 3]]) {
        let mut g = self.slot.0.lock().unwrap();
        // only the newest frame matters; an older one is simply replaced
        g.frame = Some(colors.to_vec());
        self.slot.1.notify_one();
    }

    fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn shutdown(&mut self, wait: Duration) {
        {
            let mut g = self.slot.0.lock().unwrap();
            g.stop = true;
            self.slot.1.notify_all();
        }
        let deadline = Instant::now() + wait;
        while !self.stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Threaded {
    fn drop(&mut self) {
        // Don't join: closing a session can take a moment and the pipeline
        // must not stall. The thread ends on its own.
        if !self.stopped.load(Ordering::Relaxed) {
            let mut g = self.slot.0.lock().unwrap();
            g.stop = true;
            self.slot.1.notify_all();
        }
    }
}

// ------------------------------------------------------------------ helpers

/// Stretch `src` to `n` colours (nearest neighbour). Used when a device has a
/// different number of LEDs than the zone layout provides.
pub fn resample(src: &[[u8; 3]], n: usize) -> Vec<[u8; 3]> {
    if n == 0 {
        return Vec::new();
    }
    if src.is_empty() {
        return vec![[0; 3]; n];
    }
    if src.len() == n {
        return src.to_vec();
    }
    (0..n)
        .map(|i| src[((i * 2 + 1) * src.len() / (n * 2)).min(src.len() - 1)])
        .collect()
}

/// HTTP agent for LAN / localhost APIs: short timeouts, never via a proxy,
/// and HTTP error codes come back as normal responses.
pub fn http_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .proxy(None)
        .http_status_as_error(false)
        .build()
        .into()
}

/// Standard base64 (with padding), enough for the Govee payloads.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn resample_keeps_ends() {
        let src = [[1, 0, 0], [2, 0, 0], [3, 0, 0], [4, 0, 0]];
        assert_eq!(resample(&src, 4), src.to_vec());
        let up = resample(&src, 8);
        assert_eq!(up.first(), Some(&[1, 0, 0]));
        assert_eq!(up.last(), Some(&[4, 0, 0]));
        // each output LED takes the source LED at its centre
        assert_eq!(resample(&src, 2), vec![[2, 0, 0], [4, 0, 0]]);
        assert_eq!(resample(&[], 3), vec![[0; 3]; 3]);
    }

    struct Flaky {
        fails: usize,
        written: Arc<Mutex<Vec<usize>>>,
    }

    struct Closer(Arc<AtomicBool>);

    impl Driver for Closer {
        fn connect(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn write(&mut self, _: &[[u8; 3]]) -> Result<(), String> {
            Ok(())
        }
        fn close(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn shutdown_closes_the_session() {
        let closed = Arc::new(AtomicBool::new(false));
        let mut out = Threaded::spawn(Closer(closed.clone()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while out.status() != Status::Ok && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        out.shutdown(Duration::from_secs(2));
        assert!(closed.load(Ordering::Relaxed));
    }

    impl Driver for Flaky {
        fn connect(&mut self) -> Result<(), String> {
            if self.fails > 0 {
                self.fails -= 1;
                Err("not yet".into())
            } else {
                Ok(())
            }
        }
        fn write(&mut self, colors: &[[u8; 3]]) -> Result<(), String> {
            self.written.lock().unwrap().push(colors.len());
            Ok(())
        }
    }

    #[test]
    fn threaded_reconnects_and_delivers() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut out = Threaded::spawn(Flaky {
            fails: 1,
            written: written.clone(),
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while out.status() != Status::Ok && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(out.status(), Status::Ok);
        out.send(&[[1, 2, 3]; 7]);
        while written.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(written.lock().unwrap().first(), Some(&7));
    }
}
