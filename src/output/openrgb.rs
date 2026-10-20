// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! OpenRGB SDK client (TCP, default port 6742).
//!
//! OpenRGB talks to keyboards, mice, RAM, fans, mainboards, … of almost every
//! vendor. We use its network SDK: list the controllers, switch the chosen one
//! to its "Direct" mode and stream `UpdateLeds` packets.
//!
//! The client never requests a protocol version, so the server answers in
//! protocol version 0. That format has the fewest optional fields and every
//! OpenRGB release still speaks it.
//!
//! Packet: "ORGB" | device index u32 | packet id u32 | payload size u32 | payload
//! (all little-endian). Written from the public protocol description.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::config::DeviceCfg;

pub const DEFAULT_PORT: u16 = 6742;

const MAGIC: &[u8; 4] = b"ORGB";
const REQUEST_CONTROLLER_COUNT: u32 = 0;
const REQUEST_CONTROLLER_DATA: u32 = 1;
const SET_CLIENT_NAME: u32 = 50;
const UPDATE_LEDS: u32 = 1050;
const SET_CUSTOM_MODE: u32 = 1100;
const UPDATE_MODE: u32 = 1101;

/// One RGB controller as reported by the server.
#[derive(Debug, Clone, PartialEq)]
pub struct Controller {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub location: String,
    pub led_count: usize,
    /// index and serialized data of the "Direct" mode, if the device has one
    pub direct_mode: Option<(i32, Vec<u8>)>,
}

impl Controller {
    /// Stable identifier stored in the config (indices change between runs).
    pub fn target(&self) -> String {
        format!("{}@{}", self.name, self.location)
    }
}

pub fn packet(device: u32, id: u32, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(16 + payload.len());
    p.extend_from_slice(MAGIC);
    p.extend_from_slice(&device.to_le_bytes());
    p.extend_from_slice(&id.to_le_bytes());
    p.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    p.extend_from_slice(payload);
    p
}

/// `UpdateLeds` payload: size u32 | count u16 | count × (r, g, b, 0)
pub fn update_leds_payload(colors: &[[u8; 3]]) -> Vec<u8> {
    let n = colors.len().min(u16::MAX as usize);
    let size = 4 + 2 + 4 * n;
    let mut p = Vec::with_capacity(size);
    p.extend_from_slice(&(size as u32).to_le_bytes());
    p.extend_from_slice(&(n as u16).to_le_bytes());
    for c in &colors[..n] {
        p.extend_from_slice(&[c[0], c[1], c[2], 0]);
    }
    p
}

// ------------------------------------------------------------------ parsing

struct Cur<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or("overflow")?;
        let s = self
            .b
            .get(self.pos..end)
            .ok_or("controller data too short")?;
        self.pos = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, String> {
        let len = self.u16()? as usize;
        let raw = self.take(len)?;
        let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
        Ok(String::from_utf8_lossy(raw).into_owned())
    }
}

/// Parse a protocol-0 `RequestControllerData` reply.
pub fn parse_controller(index: u32, data: &[u8]) -> Result<Controller, String> {
    let mut c = Cur { b: data, pos: 0 };
    let _size = c.u32()?;
    let _device_type = c.u32()?;
    let name = c.string()?;
    let description = c.string()?;
    let _version = c.string()?;
    let _serial = c.string()?;
    let location = c.string()?;

    let num_modes = c.u16()?;
    let _active_mode = c.u32()?;
    let mut direct_mode = None;
    for i in 0..num_modes {
        let start = c.pos;
        let mode_name = c.string()?;
        // value, flags, speed min/max, colours min/max, speed, direction, colour mode
        c.take(9 * 4)?;
        let n = c.u16()? as usize;
        c.take(n * 4)?;
        if direct_mode.is_none() && mode_name.eq_ignore_ascii_case("direct") {
            direct_mode = Some((i as i32, data[start..c.pos].to_vec()));
        }
    }

    let num_zones = c.u16()?;
    for _ in 0..num_zones {
        let _zone_name = c.string()?;
        c.take(4 * 4)?; // type, leds min, leds max, leds count
        let matrix_len = c.u16()?;
        if matrix_len > 0 {
            let h = c.u32()? as usize;
            let w = c.u32()? as usize;
            c.take(h.saturating_mul(w).saturating_mul(4))?;
        }
    }

    let num_leds = c.u16()?;
    for _ in 0..num_leds {
        let _led_name = c.string()?;
        c.take(4)?;
    }
    let num_colors = c.u16()? as usize;
    c.take(num_colors * 4)?;

    Ok(Controller {
        index,
        name,
        description,
        location,
        led_count: num_colors.max(num_leds as usize),
        direct_mode,
    })
}

// ------------------------------------------------------------------ connection

struct Conn {
    stream: TcpStream,
}

impl Conn {
    fn open(host: &str, port: u16, timeout: Duration) -> Result<Self, String> {
        let port = if port == 0 { DEFAULT_PORT } else { port };
        let addr: SocketAddr = (host, port)
            .to_socket_addrs()
            .map_err(|e| format!("{host}: {e}"))?
            .next()
            .ok_or_else(|| format!("{host}: no address"))?;
        let stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| {
            format!(
                "{}: {e}",
                crate::i18n::tr(
                    "OpenRGB nicht erreichbar – läuft der SDK-Server?",
                    "OpenRGB not reachable – is the SDK server running?"
                )
            )
        })?;
        stream.set_nodelay(true).ok();
        stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(3))).ok();
        let mut c = Self { stream };
        c.send(0, SET_CLIENT_NAME, b"WLED Ambilight\0")?;
        Ok(c)
    }

    fn send(&mut self, device: u32, id: u32, payload: &[u8]) -> Result<(), String> {
        self.stream
            .write_all(&packet(device, id, payload))
            .map_err(|e| format!("OpenRGB: {e}"))
    }

    /// Read packets until one with `id` arrives (skips notifications).
    fn reply(&mut self, id: u32) -> Result<Vec<u8>, String> {
        for _ in 0..16 {
            let mut h = [0u8; 16];
            self.stream
                .read_exact(&mut h)
                .map_err(|e| format!("OpenRGB: {e}"))?;
            if &h[0..4] != MAGIC {
                return Err("OpenRGB: bad packet header".into());
            }
            let got = u32::from_le_bytes(h[8..12].try_into().unwrap());
            let len = u32::from_le_bytes(h[12..16].try_into().unwrap()) as usize;
            if len > 16 * 1024 * 1024 {
                return Err("OpenRGB: packet too large".into());
            }
            let mut data = vec![0u8; len];
            self.stream
                .read_exact(&mut data)
                .map_err(|e| format!("OpenRGB: {e}"))?;
            if got == id {
                return Ok(data);
            }
        }
        Err("OpenRGB: no reply".into())
    }

    fn controllers(&mut self) -> Result<Vec<Controller>, String> {
        self.send(0, REQUEST_CONTROLLER_COUNT, &[])?;
        let r = self.reply(REQUEST_CONTROLLER_COUNT)?;
        let count = u32::from_le_bytes(r.get(0..4).ok_or("bad count")?.try_into().unwrap());
        let mut out = Vec::new();
        for i in 0..count.min(256) {
            self.send(i, REQUEST_CONTROLLER_DATA, &[])?;
            let data = self.reply(REQUEST_CONTROLLER_DATA)?;
            out.push(parse_controller(i, &data)?);
        }
        Ok(out)
    }
}

/// All controllers of an OpenRGB server (for discovery).
pub fn list(host: &str, port: u16, timeout: Duration) -> Result<Vec<Controller>, String> {
    Conn::open(host, port, timeout)?.controllers()
}

// ------------------------------------------------------------------ driver

pub struct Driver {
    host: String,
    port: u16,
    target: String,
    conn: Option<Conn>,
    controller: Option<Controller>,
}

impl Driver {
    pub fn new(dev: &DeviceCfg) -> Self {
        Self {
            host: dev.host.clone(),
            port: dev.port,
            target: dev.target.clone(),
            conn: None,
            controller: None,
        }
    }
}

/// Find the configured controller: exact "name@location", else by name.
pub fn find<'a>(list: &'a [Controller], target: &str) -> Option<&'a Controller> {
    let name = target.split('@').next().unwrap_or(target);
    list.iter()
        .find(|c| c.target() == target)
        .or_else(|| list.iter().find(|c| c.name == name))
}

impl super::Driver for Driver {
    fn connect(&mut self) -> Result<(), String> {
        let mut conn = Conn::open(&self.host, self.port, Duration::from_millis(1500))?;
        let list = conn.controllers()?;
        let ctrl = find(&list, &self.target).cloned().ok_or_else(|| {
            format!(
                "{} „{}“",
                crate::i18n::tr(
                    "OpenRGB kennt das Gerät nicht mehr:",
                    "OpenRGB no longer lists"
                ),
                self.target
            )
        })?;
        // Direct mode lets us drive every LED; SetCustomMode is the fallback
        // for devices without one.
        match &ctrl.direct_mode {
            Some((idx, mode)) => {
                let mut p = Vec::with_capacity(8 + mode.len());
                p.extend_from_slice(&((8 + mode.len()) as u32).to_le_bytes());
                p.extend_from_slice(&idx.to_le_bytes());
                p.extend_from_slice(mode);
                conn.send(ctrl.index, UPDATE_MODE, &p)?;
            }
            None => conn.send(ctrl.index, SET_CUSTOM_MODE, &[])?,
        }
        self.conn = Some(conn);
        self.controller = Some(ctrl);
        Ok(())
    }

    fn write(&mut self, colors: &[[u8; 3]]) -> Result<(), String> {
        let (Some(conn), Some(ctrl)) = (self.conn.as_mut(), self.controller.as_ref()) else {
            return Err("not connected".into());
        };
        let colors = super::resample(colors, ctrl.led_count);
        conn.send(ctrl.index, UPDATE_LEDS, &update_leds_payload(&colors))
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn s(out: &mut Vec<u8>, v: &str) {
        out.extend_from_slice(&((v.len() + 1) as u16).to_le_bytes());
        out.extend_from_slice(v.as_bytes());
        out.push(0);
    }

    /// Protocol-0 controller data as the server would send it.
    pub fn controller_bytes(name: &str, location: &str, leds: u16, with_matrix: bool) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0u32.to_le_bytes()); // size, filled below
        b.extend_from_slice(&5u32.to_le_bytes()); // type
        s(&mut b, name);
        s(&mut b, "a test device");
        s(&mut b, "1.0");
        s(&mut b, "SN");
        s(&mut b, location);
        b.extend_from_slice(&2u16.to_le_bytes()); // modes
        b.extend_from_slice(&0i32.to_le_bytes()); // active
        for (mode, ncol) in [("Static", 1u16), ("Direct", 0)] {
            s(&mut b, mode);
            for v in 0..9u32 {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&ncol.to_le_bytes());
            for _ in 0..ncol {
                b.extend_from_slice(&[9, 9, 9, 0]);
            }
        }
        b.extend_from_slice(&1u16.to_le_bytes()); // zones
        s(&mut b, "Keys");
        for v in [2u32, leds as u32, leds as u32, leds as u32] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        if with_matrix {
            b.extend_from_slice(&16u16.to_le_bytes());
            b.extend_from_slice(&1u32.to_le_bytes());
            b.extend_from_slice(&2u32.to_le_bytes());
            b.extend_from_slice(&[0; 8]);
        } else {
            b.extend_from_slice(&0u16.to_le_bytes());
        }
        b.extend_from_slice(&leds.to_le_bytes());
        for i in 0..leds {
            s(&mut b, &format!("Key {i}"));
            b.extend_from_slice(&(i as u32).to_le_bytes());
        }
        b.extend_from_slice(&leds.to_le_bytes());
        b.extend(std::iter::repeat_n(0u8, leds as usize * 4));
        let size = b.len() as u32;
        b[0..4].copy_from_slice(&size.to_le_bytes());
        b
    }

    #[test]
    fn parses_controller_data() {
        let b = controller_bytes("Corsair K70", "HID: /dev/x", 104, true);
        let c = parse_controller(3, &b).unwrap();
        assert_eq!(c.name, "Corsair K70");
        assert_eq!(c.location, "HID: /dev/x");
        assert_eq!(c.led_count, 104);
        assert_eq!(c.direct_mode.as_ref().map(|m| m.0), Some(1));
        assert_eq!(c.target(), "Corsair K70@HID: /dev/x");
        assert!(parse_controller(0, &b[..b.len() - 3]).is_err());
    }

    #[test]
    fn update_leds_layout() {
        let p = update_leds_payload(&[[1, 2, 3], [4, 5, 6]]);
        assert_eq!(p, vec![14, 0, 0, 0, 2, 0, 1, 2, 3, 0, 4, 5, 6, 0]);
    }

    /// (device index, packet id, payload) of every packet the server received
    type Seen = Arc<Mutex<Vec<(u32, u32, Vec<u8>)>>>;

    /// Minimal fake OpenRGB server: answers count/data, records the rest.
    fn fake_server(controllers: Vec<Vec<u8>>) -> (u16, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            loop {
                let mut h = [0u8; 16];
                if sock.read_exact(&mut h).is_err() {
                    return;
                }
                let dev = u32::from_le_bytes(h[4..8].try_into().unwrap());
                let id = u32::from_le_bytes(h[8..12].try_into().unwrap());
                let len = u32::from_le_bytes(h[12..16].try_into().unwrap()) as usize;
                let mut data = vec![0u8; len];
                sock.read_exact(&mut data).unwrap();
                match id {
                    REQUEST_CONTROLLER_COUNT => {
                        // an unrelated notification first, the client must skip it
                        sock.write_all(&packet(0, 100, &[])).unwrap();
                        sock.write_all(&packet(0, id, &(controllers.len() as u32).to_le_bytes()))
                            .unwrap();
                    }
                    REQUEST_CONTROLLER_DATA => {
                        sock.write_all(&packet(dev, id, &controllers[dev as usize]))
                            .unwrap();
                    }
                    _ => s2.lock().unwrap().push((dev, id, data)),
                }
            }
        });
        (port, seen)
    }

    #[test]
    fn driver_selects_direct_mode_and_streams() {
        use crate::output::Driver as _;
        let (port, seen) = fake_server(vec![
            controller_bytes("RAM", "I2C 1", 8, false),
            controller_bytes("Keyboard", "HID 2", 4, true),
        ]);
        let mut dev = DeviceCfg::of_kind(
            crate::config::DeviceKind::OpenRgb,
            "x",
            "127.0.0.1",
            "kb",
            4,
        );
        dev.port = port;
        dev.target = "Keyboard@HID 2".into();
        let mut d = Driver::new(&dev);
        d.connect().unwrap();
        d.write(&[[10, 20, 30]; 2]).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].1, SET_CLIENT_NAME);
        assert_eq!(seen[0].2, b"WLED Ambilight\0");
        let mode = seen
            .iter()
            .find(|p| p.1 == UPDATE_MODE)
            .expect("UpdateMode sent");
        assert_eq!(mode.0, 1, "second controller");
        assert_eq!(
            i32::from_le_bytes(mode.2[4..8].try_into().unwrap()),
            1,
            "Direct is mode 1"
        );
        let leds = seen
            .iter()
            .find(|p| p.1 == UPDATE_LEDS)
            .expect("UpdateLeds sent");
        assert_eq!(leds.0, 1);
        // two colours were stretched to the 4 LEDs of the keyboard
        assert_eq!(u16::from_le_bytes(leds.2[4..6].try_into().unwrap()), 4);
        assert_eq!(&leds.2[6..10], &[10, 20, 30, 0]);
    }
}
