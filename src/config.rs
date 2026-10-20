use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    /// DDP on UDP 4048 - best for many LEDs, what WLED prefers for realtime.
    Ddp,
    /// WLED UDP realtime protocol (DRGB) on UDP 21324, limited to ~490 LEDs/packet.
    Drgb,
}

impl Default for Protocol {
    fn default() -> Self {
        Protocol::Ddp
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceCfg {
    pub id: String,
    pub name: String,
    pub host: String,
    pub led_count: u16,
    pub enabled: bool,
    pub protocol: Protocol,
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
        }
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
    /// portal restore token so the screen capture starts without a dialog
    pub restore_token: Option<String>,
    pub snap: bool,
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

pub fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".config")
        });
    base.join("wled-ambilight").join("config.json")
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

    fn from_json(text: &str) -> Result<Self, serde_json::Error> {
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
                let id = d.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
                if flip_devices.contains(&id) {
                    d["reverse"] = serde_json::Value::Bool(true);
                }
            }
        }
        serde_json::from_value(v)
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    eprintln!("could not write {}: {e}", path.display());
                }
            }
            Err(e) => eprintln!("could not serialise config: {e}"),
        }
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
