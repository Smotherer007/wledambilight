// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Talking to WLED: discovery (mDNS + subnet sweep), `/json/info` probing and
//! the two realtime protocols (DDP and DRGB).

use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Protocol;

#[derive(Debug, Clone)]
pub struct WledInfo {
    pub host: String,
    pub name: String,
    pub leds: u16,
    pub version: String,
    pub arch: String,
}

fn http_get(host: &str, path: &str, timeout: Duration) -> Option<String> {
    let addr = format!("{host}:80");
    let sock: SocketAddr = addr.to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&sock, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let req = format!("GET {path} HTTP/1.0\r\nHost: {host}\r\nAccept: */*\r\n\r\n");
    stream.write_all(req.as_bytes()).ok()?;
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        use std::io::Read;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 64 * 1024 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let i = text.find("\r\n\r\n")?;
    Some(text[i + 4..].to_string())
}

/// Ask a single WLED device for its JSON info.
pub fn probe(host: &str, timeout: Duration) -> Option<WledInfo> {
    let body = http_get(host, "/json/info", timeout)?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    // a random HTTP server would not answer with WLED's json
    if v.get("ver").is_none() && v.get("leds").is_none() {
        return None;
    }
    let leds = v
        .get("leds")
        .and_then(|l| l.get("count"))
        .and_then(|c| c.as_u64())
        .unwrap_or(0) as u16;
    Some(WledInfo {
        host: host.to_string(),
        name: v
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("WLED")
            .to_string(),
        leds,
        version: v
            .get("ver")
            .and_then(|n| n.as_str())
            .unwrap_or("?")
            .to_string(),
        arch: v
            .get("arch")
            .and_then(|n| n.as_str())
            .unwrap_or("?")
            .to_string(),
    })
}

/// Local IPv4 address used for the default route (no packet is sent).
pub fn local_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    Some(sock.local_addr().ok()?.ip())
}

/// Probe every address of the local /24 subnet for WLED (48 at a time).
/// Finds devices that do not announce themselves via mDNS.
pub fn sweep(on_found: impl Fn(WledInfo) + Send + Sync + 'static) {
    let Some(IpAddr::V4(v4)) = local_ip() else {
        return;
    };
    let o = v4.octets();
    let hosts: Vec<String> = (1..=254)
        .filter(|last| *last != o[3])
        .map(|last| format!("{}.{}.{}.{}", o[0], o[1], o[2], last))
        .collect();
    let queue = Arc::new(Mutex::new(hosts));
    let on_found = Arc::new(on_found);
    let mut handles = Vec::new();
    for _ in 0..48 {
        let queue = queue.clone();
        let on_found = on_found.clone();
        handles.push(std::thread::spawn(move || loop {
            let Some(host) = queue.lock().unwrap().pop() else {
                break;
            };
            if let Some(info) = probe(&host, Duration::from_millis(350)) {
                on_found(info);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
}

/// mDNS browse for several service types at once (pure Rust, every OS).
/// Returns (service type, IPv4 address, instance name) for `wait`.
pub fn mdns(services: &[&str], wait: Duration) -> Vec<(String, String, String)> {
    let Ok(daemon) = mdns_sd::ServiceDaemon::new() else {
        return Vec::new();
    };
    let receivers: Vec<_> = services
        .iter()
        .filter_map(|s| daemon.browse(s).ok().map(|rx| (s.to_string(), rx)))
        .collect();
    let deadline = Instant::now() + wait;
    let mut out: Vec<(String, String, String)> = Vec::new();
    while Instant::now() < deadline {
        let mut idle = true;
        for (service, rx) in &receivers {
            while let Ok(ev) = rx.try_recv() {
                idle = false;
                if let mdns_sd::ServiceEvent::ServiceResolved(info) = ev {
                    let instance = info
                        .fullname
                        .strip_suffix(&format!(".{service}"))
                        .unwrap_or(&info.fullname)
                        .to_string();
                    for addr in info.addresses.iter() {
                        let ip = addr.to_ip_addr();
                        if ip.is_ipv4() {
                            out.push((service.clone(), ip.to_string(), instance.clone()));
                        }
                    }
                }
            }
        }
        if idle {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    for s in services {
        let _ = daemon.stop_browse(s);
    }
    let _ = daemon.shutdown();
    out.sort();
    out.dedup();
    out
}

/// Probe one host in the background (manual "add by address").
pub fn probe_in_background(host: String) -> Arc<Mutex<Option<Result<WledInfo, String>>>> {
    let out = Arc::new(Mutex::new(None));
    let o = out.clone();
    std::thread::spawn(move || {
        let host = host
            .trim()
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string();
        let res = probe(&host, Duration::from_millis(1500)).ok_or(host);
        *o.lock().unwrap() = Some(res);
    });
    out
}

pub struct DdpSender {
    sock: UdpSocket,
    seq: u8,
}

impl DdpSender {
    pub fn new(host: &str) -> Option<Self> {
        let addr = (host, 4048u16).to_socket_addrs().ok()?.next()?;
        Self::connect_to(addr)
    }

    /// Sender for an explicit address (the tests use a local port).
    fn connect_to(addr: SocketAddr) -> Option<Self> {
        let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
        sock.connect(addr).ok()?;
        sock.set_nonblocking(true).ok()?;
        Some(Self { sock, seq: 0 })
    }

    pub fn send(&mut self, colors: &[[u8; 3]]) {
        // 480 LEDs per packet keeps the datagram below the ethernet MTU
        const CHUNK: usize = 480;
        let mut offset = 0usize;
        while offset < colors.len() {
            let n = (colors.len() - offset).min(CHUNK);
            let last = offset + n >= colors.len();
            let mut pkt = Vec::with_capacity(10 + n * 3);
            // version 1; PUSH only on the last packet so WLED shows the whole
            // frame at once instead of tearing at every 480-LED boundary
            pkt.push(if last { 0x41 } else { 0x40 });
            // sequence 1..=15 (0 means "unused" in DDP)
            pkt.push((self.seq % 15) + 1);
            pkt.push(0x0B); // RGB24
            pkt.push(0x01); // destination id
            pkt.extend_from_slice(&((offset as u32) * 3).to_be_bytes());
            pkt.extend_from_slice(&((n as u16) * 3).to_be_bytes());
            for c in &colors[offset..offset + n] {
                pkt.extend_from_slice(c);
            }
            let _ = self.sock.send(&pkt);
            offset += n;
        }
        self.seq = self.seq.wrapping_add(1);
    }
}

pub struct DrgbSender {
    sock: UdpSocket,
}

impl DrgbSender {
    pub fn new(host: &str) -> Option<Self> {
        let addr = (host, 21324u16).to_socket_addrs().ok()?.next()?;
        let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
        sock.connect(addr).ok()?;
        sock.set_nonblocking(true).ok()?;
        Some(Self { sock })
    }

    pub fn send(&mut self, colors: &[[u8; 3]]) {
        let mut pkt = Vec::with_capacity(6 + colors.len() * 3);
        pkt.extend_from_slice(b"DRGB");
        pkt.push(2); // seconds until WLED falls back to its own effect
        for c in colors.iter().take(DRGB_MAX) {
            pkt.extend_from_slice(c);
        }
        let _ = self.sock.send(&pkt);
    }
}

/// DRGB fits 490 LEDs into one datagram.
pub const DRGB_MAX: usize = 490;

pub struct Sender {
    host: String,
    protocol: Protocol,
    ddp: Option<DdpSender>,
    drgb: Option<DrgbSender>,
}

impl Sender {
    pub fn new(host: &str, protocol: Protocol) -> Self {
        Self {
            host: host.to_string(),
            protocol,
            ddp: None,
            drgb: None,
        }
    }

    pub fn send(&mut self, colors: &[[u8; 3]]) {
        match self.protocol {
            Protocol::Ddp => {
                if self.ddp.is_none() {
                    self.ddp = DdpSender::new(&self.host);
                }
                if let Some(s) = self.ddp.as_mut() {
                    s.send(colors);
                }
            }
            Protocol::Drgb => {
                if self.drgb.is_none() {
                    self.drgb = DrgbSender::new(&self.host);
                }
                if let Some(s) = self.drgb.as_mut() {
                    s.send(colors);
                }
            }
        }
    }
}

/// Build the DDP datagrams for one frame (exposed for tests).
#[cfg(test)]
fn ddp_packets(colors: &[[u8; 3]]) -> Vec<Vec<u8>> {
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = sock.local_addr().unwrap().port();
    sock.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    // connect straight to the test port: macOS refuses to re-connect a
    // connected UDP socket (EISCONN), Linux allows it
    let mut tx = DdpSender::connect_to(SocketAddr::from(([127, 0, 0, 1], port))).unwrap();
    tx.send(colors);
    let mut out = Vec::new();
    let mut buf = [0u8; 2048];
    while let Ok(n) = sock.recv(&mut buf) {
        out.push(buf[..n].to_vec());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddp_splits_and_pushes_only_the_last_packet() {
        let colors = vec![[1u8, 2, 3]; 1000];
        let pkts = ddp_packets(&colors);
        assert_eq!(pkts.len(), 3);
        assert_eq!(pkts[0][0], 0x40);
        assert_eq!(pkts[1][0], 0x40);
        assert_eq!(pkts[2][0], 0x41);
        // offsets in bytes: 0, 1440, 2880
        let off = |p: &Vec<u8>| u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
        assert_eq!(off(&pkts[0]), 0);
        assert_eq!(off(&pkts[1]), 480 * 3);
        assert_eq!(off(&pkts[2]), 960 * 3);
        assert_eq!(u16::from_be_bytes([pkts[2][8], pkts[2][9]]), 40 * 3);
        assert!(pkts.iter().all(|p| (1..=15).contains(&p[1])));
    }
}
