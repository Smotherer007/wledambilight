// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Govee LAN devices.
//!
//! Discovery and on/off use Govee's documented LAN API (JSON over UDP):
//! scan on multicast 239.255.255.250:4001, replies arrive on port 4002,
//! commands go to the device on port 4003. "LAN Control" has to be switched on
//! for the device in the Govee Home app.
//!
//! Per-segment colours use the realtime mode Govee's own desktop app uses for
//! screen sync ("razer"/DreamView). It is not officially documented; the frame
//! layout below follows the community reverse engineering:
//!
//! ```text
//! BB 00 <len> B0 <stretch> <n> (R G B) × n <xor of all previous bytes>
//! len = 2 + 3n, stretch 0 = one colour per segment
//! ```
//!
//! The frame is base64-encoded into `{"msg":{"cmd":"razer","data":{"pt":…}}}`.
//! Realtime mode is switched on/off with `BB 00 01 B1 01|00 <xor>`.

use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use crate::config::DeviceCfg;

pub const COMMAND_PORT: u16 = 4003;
const SCAN_ADDR: (Ipv4Addr, u16) = (Ipv4Addr::new(239, 255, 255, 250), 4001);
const REPLY_PORT: u16 = 4002;

/// Segments of a typical Govee strip / TV backlight. Adjustable per device.
pub const DEFAULT_SEGMENTS: u16 = 10;

fn with_checksum(mut p: Vec<u8>) -> Vec<u8> {
    let x = p.iter().fold(0u8, |a, b| a ^ b);
    p.push(x);
    p
}

/// Switch realtime ("razer") mode on or off.
pub fn realtime_packet(on: bool) -> Vec<u8> {
    with_checksum(vec![0xBB, 0x00, 0x01, 0xB1, on as u8])
}

/// One realtime frame with one colour per segment (at most 84 segments).
pub fn frame_packet(colors: &[[u8; 3]]) -> Vec<u8> {
    let n = colors.len().min(84);
    let mut p = Vec::with_capacity(7 + 3 * n);
    p.extend_from_slice(&[0xBB, 0x00, (2 + 3 * n) as u8, 0xB0, 0x00, n as u8]);
    for c in &colors[..n] {
        p.extend_from_slice(c);
    }
    with_checksum(p)
}

pub fn razer_json(packet: &[u8]) -> String {
    format!(
        r#"{{"msg":{{"cmd":"razer","data":{{"pt":"{}"}}}}}}"#,
        super::base64(packet)
    )
}

/// A device that answered the LAN scan.
#[derive(Debug, Clone)]
pub struct Found {
    pub ip: String,
    pub mac: String,
    pub sku: String,
}

/// Multicast scan. Needs UDP port 4002 to be free on this computer.
pub fn scan(wait: Duration) -> Vec<Found> {
    let Ok(listen) = UdpSocket::bind(("0.0.0.0", REPLY_PORT)) else {
        return Vec::new();
    };
    listen
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok();
    if let Ok(tx) = UdpSocket::bind("0.0.0.0:0") {
        tx.set_multicast_ttl_v4(2).ok();
        let msg = r#"{"msg":{"cmd":"scan","data":{"account_topic":"reserve"}}}"#;
        let _ = tx.send_to(msg.as_bytes(), SCAN_ADDR);
    }
    let deadline = Instant::now() + wait;
    let mut out: Vec<Found> = Vec::new();
    let mut buf = [0u8; 2048];
    while Instant::now() < deadline {
        let Ok((n, _)) = listen.recv_from(&mut buf) else {
            continue;
        };
        if let Some(f) = parse_scan_reply(&buf[..n]) {
            if !out.iter().any(|o| o.mac == f.mac) {
                out.push(f);
            }
        }
    }
    out
}

pub fn parse_scan_reply(data: &[u8]) -> Option<Found> {
    let v: serde_json::Value = serde_json::from_slice(data).ok()?;
    let d = v.get("msg")?.get("data")?;
    Some(Found {
        ip: d.get("ip")?.as_str()?.to_string(),
        mac: d
            .get("device")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        sku: d
            .get("sku")
            .and_then(|x| x.as_str())
            .unwrap_or("Govee")
            .to_string(),
    })
}

pub struct Driver {
    host: String,
    sock: Option<(UdpSocket, SocketAddr)>,
    last_enable: Instant,
}

impl Driver {
    pub fn new(dev: &DeviceCfg) -> Self {
        Self {
            host: dev.host.clone(),
            sock: None,
            last_enable: Instant::now(),
        }
    }

    fn send(&self, msg: &str) -> Result<(), String> {
        let (sock, addr) = self.sock.as_ref().ok_or("not connected")?;
        sock.send_to(msg.as_bytes(), addr)
            .map(|_| ())
            .map_err(|e| format!("Govee: {e}"))
    }
}

impl super::Driver for Driver {
    fn connect(&mut self) -> Result<(), String> {
        let addr = (self.host.as_str(), COMMAND_PORT)
            .to_socket_addrs()
            .map_err(|e| format!("{}: {e}", self.host))?
            .next()
            .ok_or_else(|| format!("{}: no address", self.host))?;
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        self.sock = Some((sock, addr));
        self.send(r#"{"msg":{"cmd":"turn","data":{"value":1}}}"#)?;
        self.send(&razer_json(&realtime_packet(true)))?;
        self.last_enable = Instant::now();
        Ok(())
    }

    fn write(&mut self, colors: &[[u8; 3]]) -> Result<(), String> {
        self.send(&razer_json(&frame_packet(colors)))
    }

    fn keepalive(&mut self) -> Result<(), String> {
        // the device can drop out of realtime mode (app, power cycle): re-arm
        if self.last_enable.elapsed() >= Duration::from_secs(5) {
            self.last_enable = Instant::now();
            self.send(&razer_json(&realtime_packet(true)))?;
        }
        Ok(())
    }

    fn close(&mut self) {
        let _ = self.send(&razer_json(&realtime_packet(false)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realtime_switch_matches_known_packets() {
        // the base64 strings used by every public implementation
        assert_eq!(super::super::base64(&realtime_packet(true)), "uwABsQEK");
        assert_eq!(super::super::base64(&realtime_packet(false)), "uwABsQAL");
    }

    #[test]
    fn frame_layout() {
        let p = frame_packet(&[[1, 2, 3]; 10]);
        assert_eq!(&p[..6], &[0xBB, 0x00, 0x20, 0xB0, 0x00, 10]);
        assert_eq!(p.len(), 6 + 30 + 1);
        assert_eq!(
            p.iter().fold(0u8, |a, b| a ^ b),
            0,
            "xor over all bytes incl. checksum is 0"
        );
        let j = razer_json(&p);
        assert!(
            j.starts_with(r#"{"msg":{"cmd":"razer","data":{"pt":"uwAgsAAK"#),
            "{j}"
        );
    }

    #[test]
    fn scan_reply() {
        let r = br#"{"msg":{"cmd":"scan","data":{"ip":"192.168.1.23","device":"1F:80:C5:32:32:36:72:4E","sku":"H6199","bleVersionHard":"3.01.01","bleVersionSoft":"1.03.01","wifiVersionHard":"1.00.10","wifiVersionSoft":"1.02.03"}}}"#;
        let f = parse_scan_reply(r).unwrap();
        assert_eq!(f.ip, "192.168.1.23");
        assert_eq!(f.sku, "H6199");
        assert!(parse_scan_reply(b"nope").is_none());
    }
}
