// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! "Search the network": finds every supported device kind in parallel.
//!
//! | Kind     | How                                                   |
//! |----------|-------------------------------------------------------|
//! | WLED     | mDNS `_wled._tcp`, then a sweep of the local /24      |
//! | Nanoleaf | mDNS `_nanoleafapi._tcp`                              |
//! | Govee    | LAN API multicast scan                                |
//! | OpenRGB  | SDK server on this computer (127.0.0.1:6742)          |
//! | Razer    | Chroma SDK server of Synapse on this computer         |

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::{DeviceCfg, DeviceKind};
use crate::output::{govee, nanoleaf, openrgb, razer};
use crate::wled;

const WLED_SERVICE: &str = "_wled._tcp.local.";
const NANOLEAF_SERVICE: &str = "_nanoleafapi._tcp.local.";

/// A device that can be added with one click.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub kind: DeviceKind,
    /// unique device id (see [`Found::into_device`])
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub target: String,
    pub leds: u16,
    /// second line in the list: address, model, firmware, …
    pub detail: String,
}

impl Found {
    pub fn wled(info: &wled::WledInfo) -> Self {
        Self {
            kind: DeviceKind::Wled,
            id: info.host.clone(),
            name: info.name.clone(),
            host: info.host.clone(),
            port: 0,
            target: String::new(),
            leds: info.leds,
            detail: format!("{} · v{} · {}", info.host, info.version, info.arch),
        }
    }

    pub fn openrgb(host: &str, port: u16, c: &openrgb::Controller) -> Self {
        Self {
            kind: DeviceKind::OpenRgb,
            id: format!("openrgb:{host}:{port}:{}", c.target()),
            name: c.name.clone(),
            host: host.to_string(),
            port,
            target: c.target(),
            leds: c.led_count.min(u16::MAX as usize) as u16,
            detail: if c.description.is_empty() {
                c.location.clone()
            } else {
                c.description.clone()
            },
        }
    }

    pub fn razer() -> Self {
        Self {
            kind: DeviceKind::Razer,
            id: "razer:chroma".into(),
            name: "Razer Chroma".into(),
            host: "127.0.0.1".into(),
            port: razer::PORT,
            target: String::new(),
            leds: razer::DEFAULT_COLUMNS,
            detail: crate::i18n::tr(
                "alle Razer-Geräte und Chroma Connect",
                "all Razer devices and Chroma Connect",
            )
            .into(),
        }
    }

    pub fn govee(ip: &str, mac: &str, sku: &str) -> Self {
        let key = if mac.is_empty() { ip } else { mac };
        Self {
            kind: DeviceKind::Govee,
            id: format!("govee:{key}"),
            name: format!("Govee {sku}"),
            host: ip.to_string(),
            port: govee::COMMAND_PORT,
            target: sku.to_string(),
            leds: govee::DEFAULT_SEGMENTS,
            detail: ip.to_string(),
        }
    }

    pub fn nanoleaf(ip: &str, name: &str) -> Self {
        Self {
            kind: DeviceKind::Nanoleaf,
            id: format!("nanoleaf:{ip}"),
            name: if name.is_empty() {
                "Nanoleaf".into()
            } else {
                name.to_string()
            },
            host: ip.to_string(),
            port: nanoleaf::API_PORT,
            target: String::new(),
            leds: 0,
            detail: ip.to_string(),
        }
    }

    pub fn into_device(self) -> DeviceCfg {
        let mut d = DeviceCfg::of_kind(self.kind, self.id, self.host, self.name, self.leds.max(1));
        d.port = self.port;
        d.target = self.target;
        // TCP/HTTP devices and Govee firmware don't keep up with 50+ fps
        d.max_fps = match self.kind {
            DeviceKind::Razer | DeviceKind::Govee => 30,
            DeviceKind::Nanoleaf => 10,
            _ => 0,
        };
        d
    }
}

pub struct Scan {
    pub found: Arc<Mutex<Vec<Found>>>,
    pending: Arc<AtomicUsize>,
}

impl Scan {
    pub fn done(&self) -> bool {
        self.pending.load(Ordering::Relaxed) == 0
    }
}

fn push(found: &Mutex<Vec<Found>>, f: Found) {
    let mut v = found.lock().unwrap();
    if !v.iter().any(|x| x.id == f.id) {
        v.push(f);
    }
}

fn job(scan: &Scan, work: impl FnOnce(Arc<Mutex<Vec<Found>>>) + Send + 'static) {
    let found = scan.found.clone();
    let pending = scan.pending.clone();
    pending.fetch_add(1, Ordering::Relaxed);
    std::thread::spawn(move || {
        work(found);
        pending.fetch_sub(1, Ordering::Relaxed);
    });
}

/// Search for every device kind; results trickle into `found`.
pub fn search() -> Scan {
    let scan = Scan {
        found: Arc::new(Mutex::new(Vec::new())),
        pending: Arc::new(AtomicUsize::new(0)),
    };
    job(&scan, |found| {
        let hits = wled::mdns(
            &[WLED_SERVICE, NANOLEAF_SERVICE],
            Duration::from_millis(1500),
        );
        for (service, ip, name) in &hits {
            if service == NANOLEAF_SERVICE {
                push(&found, Found::nanoleaf(ip, name));
            } else if let Some(info) = wled::probe(ip, Duration::from_millis(600)) {
                push(&found, Found::wled(&info));
            }
        }
        let f = found.clone();
        wled::sweep(move |info| push(&f, Found::wled(&info)));
    });
    job(&scan, |found| {
        for g in govee::scan(Duration::from_secs(2)) {
            push(&found, Found::govee(&g.ip, &g.mac, &g.sku));
        }
    });
    job(&scan, |found| {
        if let Ok(list) = openrgb::list(
            "127.0.0.1",
            openrgb::DEFAULT_PORT,
            Duration::from_millis(400),
        ) {
            for c in &list {
                push(
                    &found,
                    Found::openrgb("127.0.0.1", openrgb::DEFAULT_PORT, c),
                );
            }
        }
    });
    job(&scan, |found| {
        if razer::available() {
            push(&found, Found::razer());
        }
    });
    scan
}

/// Controllers of an OpenRGB server on another computer (manual add).
pub fn openrgb_at(scan: &Scan, host: String, port: u16) -> Arc<Mutex<Option<String>>> {
    let error = Arc::new(Mutex::new(None));
    let e = error.clone();
    job(scan, move |found| {
        match openrgb::list(&host, port, Duration::from_millis(1500)) {
            Ok(list) if list.is_empty() => {
                *e.lock().unwrap() = Some(format!("OpenRGB {host}: 0 controllers"));
            }
            Ok(list) => {
                for c in &list {
                    push(&found, Found::openrgb(&host, port, c));
                }
            }
            Err(err) => *e.lock().unwrap() = Some(err),
        }
    });
    error
}

/// An empty, finished scan to collect manual results into.
pub fn empty() -> Scan {
    Scan {
        found: Arc::new(Mutex::new(Vec::new())),
        pending: Arc::new(AtomicUsize::new(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_per_kind() {
        let a = Found::govee("10.0.0.5", "AA:BB", "H6199");
        let b = Found::nanoleaf("10.0.0.5", "Shapes");
        assert_ne!(a.id, b.id);
        let d = a.into_device();
        assert_eq!(d.kind, DeviceKind::Govee);
        assert_eq!(d.led_count, govee::DEFAULT_SEGMENTS);
        assert_eq!(d.max_fps, 30);
        // unpaired Nanoleaf starts with one placeholder panel
        assert_eq!(b.into_device().led_count, 1);
    }
}
