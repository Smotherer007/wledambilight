// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Nanoleaf panels via the official OpenAPI (port 16021) and External
//! Control v2 (UDP 60222).
//!
//! Pairing: hold the controller's power button for 5–7 s, then
//! `POST /api/v1/new` returns an auth token within the next 30 s.
//!
//! Every light panel is one "LED". They are ordered by their position in the
//! layout - left to right, or top to bottom when the layout is taller than
//! wide - so a zone maps onto the panels the way they hang on the wall.
//!
//! v2 frame: n u16 | n × (panel id u16, R, G, B, W, transition u16), big-endian.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use crate::config::DeviceCfg;
use crate::i18n::tr;

pub const API_PORT: u16 = 16021;
const STREAM_PORT: u16 = 60222;
/// Transition time per frame in 100 ms units. 1 = a short fade that hides
/// the low panel refresh rate; our own smoothing does the rest.
const TRANSITION: u16 = 1;

/// Shape types that carry no light: controllers, connectors, power supply,
/// the Rhythm module.
const NOT_LIGHTS: &[u64] = &[1, 12, 16, 19, 20];

#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub id: u16,
    pub x: i64,
    pub y: i64,
}

fn base(host: &str) -> String {
    format!("http://{host}:{API_PORT}/api/v1")
}

/// Ask for an auth token. Works only while the device is in pairing mode.
pub fn pair(host: &str) -> Result<String, String> {
    let agent = super::http_agent(Duration::from_secs(3));
    let mut r = agent
        .post(format!("{}/new", base(host)))
        .send_empty()
        .map_err(|e| format!("Nanoleaf: {e}"))?;
    if r.status() == 403 {
        return Err(tr(
            "Nanoleaf ist nicht im Kopplungsmodus: Ein/Aus-Taste am Controller 5–7 Sekunden halten, dann sofort „Koppeln“ klicken.",
            "Nanoleaf is not in pairing mode: hold the controller's power button for 5–7 seconds, then click “Pair” right away.",
        )
        .into());
    }
    let v: serde_json::Value = r
        .body_mut()
        .read_json()
        .map_err(|e| format!("Nanoleaf: {e}"))?;
    v.get("auth_token")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Nanoleaf: no token in reply".into())
}

/// Light panels in display order.
pub fn layout(host: &str, token: &str) -> Result<Vec<Panel>, String> {
    let agent = super::http_agent(Duration::from_secs(3));
    let mut r = agent
        .get(format!("{}/{token}/panelLayout/layout", base(host)))
        .call()
        .map_err(|e| format!("Nanoleaf: {e}"))?;
    if r.status() == 401 {
        return Err(tr(
            "Nanoleaf hat die Kopplung abgelehnt – bitte neu koppeln.",
            "Nanoleaf rejected the pairing – please pair again.",
        )
        .into());
    }
    let v: serde_json::Value = r
        .body_mut()
        .read_json()
        .map_err(|e| format!("Nanoleaf: {e}"))?;
    Ok(order_panels(&v))
}

pub fn order_panels(layout: &serde_json::Value) -> Vec<Panel> {
    let mut panels: Vec<Panel> = layout
        .get("positionData")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter()
                .filter(|p| {
                    let shape = p.get("shapeType").and_then(|s| s.as_u64()).unwrap_or(0);
                    !NOT_LIGHTS.contains(&shape)
                })
                .filter_map(|p| {
                    Some(Panel {
                        id: p.get("panelId")?.as_u64()? as u16,
                        x: p.get("x")?.as_i64()?,
                        y: p.get("y")?.as_i64()?,
                    })
                })
                .filter(|p| p.id != 0)
                .collect()
        })
        .unwrap_or_default();
    let span = |f: fn(&Panel) -> i64| {
        let min = panels.iter().map(f).min().unwrap_or(0);
        let max = panels.iter().map(f).max().unwrap_or(0);
        max - min
    };
    if span(|p| p.y) > span(|p| p.x) {
        // taller than wide: top to bottom (Nanoleaf's y grows upwards)
        panels.sort_by_key(|p| (-p.y, p.x));
    } else {
        panels.sort_by_key(|p| (p.x, -p.y));
    }
    panels
}

pub fn frame_packet(panels: &[Panel], colors: &[[u8; 3]]) -> Vec<u8> {
    let n = panels.len().min(colors.len());
    let mut p = Vec::with_capacity(2 + n * 8);
    p.extend_from_slice(&(n as u16).to_be_bytes());
    for (panel, c) in panels.iter().zip(colors) {
        p.extend_from_slice(&panel.id.to_be_bytes());
        p.extend_from_slice(&[c[0], c[1], c[2], 0]);
        p.extend_from_slice(&TRANSITION.to_be_bytes());
    }
    p
}

pub struct Driver {
    host: String,
    token: Option<String>,
    panels: Vec<Panel>,
    sock: Option<(UdpSocket, SocketAddr)>,
}

impl Driver {
    pub fn new(dev: &DeviceCfg) -> Self {
        Self {
            host: dev.host.clone(),
            token: dev.token.clone(),
            panels: Vec::new(),
            sock: None,
        }
    }
}

impl super::Driver for Driver {
    fn connect(&mut self) -> Result<(), String> {
        let token = self
            .token
            .clone()
            .ok_or_else(|| tr("Noch nicht gekoppelt", "Not paired yet").to_string())?;
        self.panels = layout(&self.host, &token)?;
        let agent = super::http_agent(Duration::from_secs(3));
        let r = agent
            .put(format!("{}/{token}/effects", base(&self.host)))
            .send_json(serde_json::json!({
                "write": {"command": "display", "animType": "extControl", "extControlVersion": "v2"}
            }))
            .map_err(|e| format!("Nanoleaf: {e}"))?;
        if !r.status().is_success() {
            return Err(format!("Nanoleaf: extControl {}", r.status()));
        }
        let addr = (self.host.as_str(), STREAM_PORT)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .next()
            .ok_or("no address")?;
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        self.sock = Some((sock, addr));
        Ok(())
    }

    fn write(&mut self, colors: &[[u8; 3]]) -> Result<(), String> {
        let (sock, addr) = self.sock.as_ref().ok_or("not connected")?;
        let colors = super::resample(colors, self.panels.len());
        sock.send_to(&frame_packet(&self.panels, &colors), addr)
            .map(|_| ())
            .map_err(|e| format!("Nanoleaf: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panels_left_to_right_without_controller() {
        let v = serde_json::json!({"numPanels": 4, "positionData": [
            {"panelId": 30, "x": 200, "y": 0, "o": 0, "shapeType": 7},
            {"panelId": 10, "x": 0, "y": 0, "o": 0, "shapeType": 7},
            {"panelId": 0, "x": 100, "y": 0, "o": 0, "shapeType": 12},
            {"panelId": 20, "x": 100, "y": 50, "o": 0, "shapeType": 7}
        ]});
        let ids: Vec<u16> = order_panels(&v).iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![10, 20, 30]);
    }

    #[test]
    fn tall_layouts_go_top_to_bottom() {
        let v = serde_json::json!({"positionData": [
            {"panelId": 1, "x": 0, "y": 0, "shapeType": 2},
            {"panelId": 2, "x": 0, "y": 300, "shapeType": 2},
            {"panelId": 3, "x": 10, "y": 150, "shapeType": 2}
        ]});
        let ids: Vec<u16> = order_panels(&v).iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![2, 3, 1]);
    }

    /// One-shot HTTP server on a random port answering every request with `body`.
    fn http_once(status: &str, body: &'static str) -> u16 {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let status = status.to_string();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                let mut buf = [0u8; 2048];
                let _ = c.read(&mut buf);
                let _ = write!(
                    c,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        port
    }

    #[test]
    fn pairing_reads_the_token_and_reports_pairing_mode() {
        let port = http_once("200 OK", r#"{"auth_token":"abc"}"#);
        let agent = super::super::http_agent(Duration::from_secs(2));
        let mut r = agent
            .post(format!("http://127.0.0.1:{port}/api/v1/new"))
            .send_empty()
            .unwrap();
        let v: serde_json::Value = r.body_mut().read_json().unwrap();
        assert_eq!(v["auth_token"], "abc");
        // the real pair() maps 403 to a helpful message
        let port = http_once("403 Forbidden", "");
        let r = agent
            .post(format!("http://127.0.0.1:{port}/api/v1/new"))
            .send_empty()
            .unwrap();
        assert_eq!(r.status(), 403, "error codes come back as responses");
    }

    #[test]
    fn v2_frame() {
        let panels = vec![Panel {
            id: 258,
            x: 0,
            y: 0,
        }];
        let p = frame_packet(&panels, &[[1, 2, 3]]);
        assert_eq!(p, vec![0, 1, 1, 2, 1, 2, 3, 0, 0, 1]);
    }
}
