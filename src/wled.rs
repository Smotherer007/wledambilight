use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};

use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    let body = match text.find("\r\n\r\n") {
        Some(i) => text[i + 4..].to_string(),
        None => return None,
    };
    Some(body)
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

pub struct Scan {
    pub found: Arc<Mutex<Vec<WledInfo>>>,
    pub done: Arc<Mutex<bool>>,
}

/// One search that does both: mDNS lookup first (instant, finds devices that
/// announce themselves), then the subnet sweep for everything else.
pub fn search() -> Scan {
    let found: Arc<Mutex<Vec<WledInfo>>> = Arc::new(Mutex::new(Vec::new()));
    let done = Arc::new(Mutex::new(false));
    let f = found.clone();
    let d = done.clone();
    std::thread::spawn(move || {
        for host in mdns_hosts() {
            if let Some(info) = probe(&host, Duration::from_millis(600)) {
                let mut v = f.lock().unwrap();
                if !v.iter().any(|x| x.host == info.host) {
                    v.push(info);
                }
            }
        }
        if let Some(IpAddr::V4(v4)) = local_ip() {
            let o = v4.octets();
            let hosts: Vec<String> = (1..=254)
                .filter(|last| *last != o[3])
                .map(|last| format!("{}.{}.{}.{}", o[0], o[1], o[2], last))
                .collect();
            let queue = Arc::new(Mutex::new(hosts));
            let mut handles = Vec::new();
            for _ in 0..48 {
                let queue = queue.clone();
                let f = f.clone();
                handles.push(std::thread::spawn(move || loop {
                    let host = {
                        let mut q = queue.lock().unwrap();
                        match q.pop() {
                            Some(h) => h,
                            None => break,
                        }
                    };
                    if let Some(info) = probe(&host, Duration::from_millis(350)) {
                        let mut v = f.lock().unwrap();
                        if !v.iter().any(|x| x.host == info.host) {
                            v.push(info);
                        }
                    }
                }));
            }
            for h in handles {
                let _ = h.join();
            }
        }
        *d.lock().unwrap() = true;
    });
    Scan { found, done }
}

/// mDNS discovery through avahi, if it is installed.
pub fn mdns_hosts() -> Vec<String> {
    let Ok(out) = std::process::Command::new("avahi-browse")
        .args(["-tpr", "_wled._tcp"])
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut hosts = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split(';').collect();
        // +;iface;proto;name;type;domain;hostname;address;port;txt
        if parts.len() > 8 && parts[0] == "=" {
            hosts.push(parts[8].to_string());
        }
    }
    hosts.sort();
    hosts.dedup();
    hosts
}

pub struct DdpSender {
    sock: UdpSocket,
    addr: SocketAddr,
    seq: u8,
}

impl DdpSender {
    pub fn new(host: &str) -> Option<Self> {
        let addr = (host, 4048u16).to_socket_addrs().ok()?.next()?;
        let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
        sock.connect(addr).ok()?;
        sock.set_nonblocking(true).ok()?;
        Some(Self { sock, addr, seq: 0 })
    }

    pub fn send(&mut self, colors: &[[u8; 3]]) {
        // 480 LEDs per packet keeps the datagram below the ethernet MTU
        const CHUNK: usize = 480;
        let mut offset = 0usize;
        while offset < colors.len() {
            let n = (colors.len() - offset).min(CHUNK);
            let mut pkt = Vec::with_capacity(10 + n * 3);
            pkt.push(0x41); // push + version 1
            pkt.push(self.seq & 0x0F);
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
        let _ = self.addr;
    }
}

pub struct DrgbSender {
    sock: UdpSocket,
    addr: SocketAddr,
}

impl DrgbSender {
    pub fn new(host: &str) -> Option<Self> {
        let addr = (host, 21324u16).to_socket_addrs().ok()?.next()?;
        let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
        sock.connect(addr).ok()?;
        sock.set_nonblocking(true).ok()?;
        Some(Self { sock, addr })
    }

    pub fn send(&mut self, colors: &[[u8; 3]]) {
        let mut pkt = Vec::with_capacity(6 + colors.len() * 3);
        pkt.extend_from_slice(b"DRGB");
        pkt.push(2); // timeout in seconds
        for c in colors.iter().take(490) {
            pkt.extend_from_slice(c);
        }
        let _ = self.sock.send(&pkt);
        let _ = self.addr;
    }
}

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


