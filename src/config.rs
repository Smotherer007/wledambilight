// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Persistent configuration: devices, zones and global settings.
//!
//! Stored as pretty JSON in the platform config directory:
//! - Linux:   `$XDG_CONFIG_HOME/wled-ambilight/config.json` (usually `~/.config/…`)
//! - macOS:   `~/Library/Application Support/wled-ambilight/config.json`
//! - Windows: `%APPDATA%\wled-ambilight\config.json`
//!
//! `--config <file>` overrides the location.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    /// DDP on UDP 4048 - best for many LEDs, what WLED prefers for realtime.
    #[default]
    Ddp,
    /// WLED UDP realtime protocol (DRGB) on UDP 21324, limited to 490 LEDs.
    Drgb,
}

/// What kind of device a [`DeviceCfg`] talks to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceKind {
    /// WLED over DDP or DRGB
    #[default]
    Wled,
    /// one controller of an OpenRGB SDK server (PC peripherals, RAM, fans, …)
    OpenRgb,
    /// Razer Chroma via the local REST API of Razer Synapse (Windows)
    Razer,
    /// Govee LAN device in realtime ("razer"/DreamView) mode
    Govee,
    /// Nanoleaf panels via External Control v2
    Nanoleaf,
}

impl DeviceKind {
    pub fn label(self) -> &'static str {
        match self {
            DeviceKind::Wled => "WLED",
            DeviceKind::OpenRgb => "OpenRGB",
            DeviceKind::Razer => "Razer Chroma",
            DeviceKind::Govee => "Govee",
            DeviceKind::Nanoleaf => "Nanoleaf",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceCfg {
    pub id: String,
    pub name: String,
    pub host: String,
    /// LEDs, segments (Govee), panels (Nanoleaf) or columns (Razer)
    pub led_count: u16,
    pub enabled: bool,
    /// WLED only
    pub protocol: Protocol,
    #[serde(default)]
    pub kind: DeviceKind,
    /// OpenRGB: TCP port of the SDK server (0 = 6742)
    #[serde(default)]
    pub port: u16,
    /// OpenRGB: "controller name@location"; Govee: model (SKU)
    #[serde(default)]
    pub target: String,
    /// Nanoleaf: auth token from pairing
    #[serde(default)]
    pub token: Option<String>,
    /// 0..1, scales everything sent to this device
    pub brightness: f32,
    /// physical strip is wired the other way round
    pub reverse: bool,
    /// BGR instead of RGB (some clones)
    pub bgr: bool,
    /// cap the update rate of this device, 0 = use the global rate
    #[serde(default)]
    pub max_fps: u32,
}

impl DeviceCfg {
    pub fn new(host: impl Into<String>, name: impl Into<String>, leds: u16) -> Self {
        let host = host.into();
        Self {
            id: host.clone(),
            name: name.into(),
            host,
            led_count: leds.max(1),
            enabled: true,
            protocol: Protocol::Ddp,
            brightness: 1.0,
            reverse: false,
            bgr: false,
            max_fps: 0,
            kind: DeviceKind::Wled,
            port: 0,
            target: String::new(),
            token: None,
        }
    }

    /// A device of another kind. `id` must be unique across all devices.
    pub fn of_kind(
        kind: DeviceKind,
        id: impl Into<String>,
        host: impl Into<String>,
        name: impl Into<String>,
        leds: u16,
    ) -> Self {
        let mut d = Self::new(host, name, leds);
        d.id = id.into();
        d.kind = kind;
        d
    }

    /// Everything that requires a new connection when it changes.
    pub fn connection_key(&self) -> String {
        format!(
            "{:?}|{}|{}|{:?}|{}|{}|{:?}",
            self.kind, self.host, self.port, self.protocol, self.target, self.led_count, self.token
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub name: String,
    /// DeviceCfg::id
    pub device: String,
    /// normalised screen coordinates 0..1 - independent of any resolution
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// how many LEDs this zone drives
    pub count: u16,
    /// true: LEDs run top -> bottom, false: left -> right
    pub vertical: bool,
    /// per zone gain
    pub gain: f32,
    pub enabled: bool,
    /// start index inside the device; None = packed after the previous zone of that device
    pub offset: Option<u16>,
    pub color: [u8; 3],
}

impl Zone {
    pub fn preset(kind: ZonePreset, device: String, count: u16, color: [u8; 3]) -> Self {
        let (name, x, y, w, h, vertical) = match kind {
            ZonePreset::Top => ("Top", 0.0, 0.0, 1.0, 0.10, false),
            ZonePreset::Bottom => ("Bottom", 0.0, 0.90, 1.0, 0.10, false),
            ZonePreset::Left => ("Left", 0.0, 0.0, 0.10, 1.0, true),
            ZonePreset::Right => ("Right", 0.90, 0.0, 0.10, 1.0, true),
            ZonePreset::Full => ("Full", 0.0, 0.0, 1.0, 1.0, false),
            ZonePreset::Custom => ("Zone", 0.25, 0.25, 0.5, 0.25, false),
        };
        Self {
            name: name.to_string(),
            device,
            x,
            y,
            w,
            h,
            count: count.max(1),
            vertical,
            gain: 1.0,
            enabled: true,
            offset: None,
            color,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZonePreset {
    Top,
    Bottom,
    Left,
    Right,
    /// whole picture (used by tests and the sampler checks)
    #[allow(dead_code)]
    Full,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// frames per second we sample and send
    pub target_fps: u32,
    /// width of the capture buffer, height follows the screen aspect
    pub capture_width: u32,
    /// temporal smoothing in ms, 0 = off
    pub smoothing_ms: f32,
    /// 0..1 blur along the strip, 0 = every LED on its own
    pub spatial: f32,
    /// max change per second (0..255 scale), 0 = unlimited
    pub slew: f32,
    /// freeze the LEDs while this window is focused (it would capture itself)
    pub pause_when_focused: bool,
    pub gamma: f32,
    pub saturation: f32,
    pub brightness: f32,
    /// Linux only: portal restore token so the capture starts without a dialog
    pub restore_token: Option<String>,
    pub snap: bool,
    /// Windows/macOS: index of the captured monitor (0 = primary)
    pub monitor: usize,
    /// "auto", "de" or "en"
    pub language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            target_fps: 50,
            capture_width: 320,
            smoothing_ms: 180.0,
            spatial: 0.6,
            slew: 900.0,
            pause_when_focused: false,
            gamma: 1.0,
            saturation: 1.0,
            brightness: 1.0,
            restore_token: None,
            snap: true,
            monitor: 0,
            language: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub devices: Vec<DeviceCfg>,
    pub zones: Vec<Zone>,
    pub settings: Settings,
}

static PATH_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Use a different config file (`--config <file>`). Call before anything loads.
pub fn set_config_path(path: PathBuf) {
    let _ = PATH_OVERRIDE.set(path);
}

pub fn config_path() -> PathBuf {
    if let Some(p) = PATH_OVERRIDE.get() {
        return p.clone();
    }
    // dirs::config_dir() is $XDG_CONFIG_HOME or ~/.config on Linux, so existing
    // installations keep their file.
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wled-ambilight")
        .join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::from_json(&text).unwrap_or_else(|e| {
                eprintln!("config {} is broken ({e}), starting fresh", path.display());
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let mut v: serde_json::Value = serde_json::from_str(text)?;
        // Zones used to carry flip_h/flip_v. A reversed strip is a property of the
        // device, so fold the old flags into it (safe for the one-zone case).
        let mut flip_devices: Vec<String> = Vec::new();
        if let Some(zones) = v.get_mut("zones").and_then(|z| z.as_array_mut()) {
            for z in zones.iter_mut() {
                let flipped = z.get("flip_h").and_then(|b| b.as_bool()).unwrap_or(false)
                    || z.get("flip_v").and_then(|b| b.as_bool()).unwrap_or(false)
                    || z.get("reverse").and_then(|b| b.as_bool()).unwrap_or(false);
                if flipped {
                    if let Some(dev) = z.get("device").and_then(|d| d.as_str()) {
                        flip_devices.push(dev.to_string());
                    }
                }
                if let Some(o) = z.as_object_mut() {
                    o.remove("flip_h");
                    o.remove("flip_v");
                    o.remove("reverse");
                }
            }
        }
        if let Some(devices) = v.get_mut("devices").and_then(|d| d.as_array_mut()) {
            for d in devices {
                let id = d
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string();
                if flip_devices.contains(&id) {
                    d["reverse"] = serde_json::Value::Bool(true);
                }
            }
        }
        serde_json::from_value(v)
    }

    /// Write the config atomically (temp file + rename), so a crash or a full
    /// disk can never leave a half written file behind.
    pub fn save(&self) {
        let path = config_path();
        if let Err(e) = self.save_to(&path) {
            eprintln!("could not write {}: {e}", path.display());
        }
    }

    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    pub fn device(&self, id: &str) -> Option<&DeviceCfg> {
        self.devices.iter().find(|d| d.id == id)
    }

    /// Which LEDs of which device every zone drives, in config order.
    /// This is exactly the packing the sampler uses.
    pub fn zone_ranges(&self) -> Vec<(u32, u32)> {
        let mut used: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
        let mut out = Vec::with_capacity(self.zones.len());
        for z in &self.zones {
            let start = z
                .offset
                .map(|o| o as u32)
                .unwrap_or_else(|| *used.get(z.device.as_str()).unwrap_or(&0));
            let end = start + z.count as u32;
            if z.enabled {
                used.insert(z.device.as_str(), end);
            }
            out.push((start, end));
        }
        out
    }

    /// LEDs already claimed on a device, ignoring `skip`.
    pub fn used_leds(&self, device_id: &str, skip: Option<usize>) -> u16 {
        self.zones
            .iter()
            .enumerate()
            .filter(|(i, z)| z.device == device_id && z.enabled && Some(*i) != skip)
            .map(|(_, z)| z.count)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_zone_flip_flags_move_to_the_device() {
        let json = r#"{
            "devices": [{"id":"a","name":"A","host":"a","led_count":10,"enabled":true,
                         "protocol":"Ddp","brightness":1.0,"reverse":false,"bgr":false}],
            "zones": [{"name":"Z","device":"a","x":0,"y":0,"w":1,"h":0.1,"count":10,
                       "vertical":false,"gain":1,"enabled":true,"offset":null,
                       "color":[1,2,3],"flip_h":true}]
        }"#;
        let cfg = Config::from_json(json).unwrap();
        assert!(cfg.devices[0].reverse);
        assert_eq!(cfg.devices[0].max_fps, 0);
        assert_eq!(cfg.settings.language, "auto");
        // configs from 0.2.0 have no kind: they are WLED devices
        assert_eq!(cfg.devices[0].kind, DeviceKind::Wled);
        assert_eq!(cfg.devices[0].token, None);
    }

    #[test]
    fn zones_pack_after_each_other() {
        let mut cfg = Config::default();
        cfg.devices.push(DeviceCfg::new("h", "d", 100));
        for kind in [ZonePreset::Top, ZonePreset::Right] {
            cfg.zones
                .push(Zone::preset(kind, "h".into(), 30, [0, 0, 0]));
        }
        assert_eq!(cfg.zone_ranges(), vec![(0, 30), (30, 60)]);
        assert_eq!(cfg.used_leds("h", None), 60);
        assert_eq!(cfg.used_leds("h", Some(0)), 30);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("wled-ambilight-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let mut cfg = Config::default();
        cfg.devices.push(DeviceCfg::new("10.0.0.2", "Desk", 42));
        cfg.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(Config::from_json(&text).unwrap(), cfg);
        let _ = std::fs::remove_dir_all(dir);
    }
}
