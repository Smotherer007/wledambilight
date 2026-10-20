// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Razer Chroma via the local REST API of Razer Synapse (Windows).
//!
//! `POST http://localhost:54235/razer/chromasdk` opens a session and returns
//! its own URI; effects are `PUT <uri>/<device>`, `PUT <uri>/heartbeat` keeps
//! the session alive and `DELETE <uri>` ends it. Only plain HTTP, no Razer
//! code is linked into the app.
//!
//! Chroma shows the effects of the most recently started app, so all Razer
//! hardware shares one session and one configured device. The device's
//! "LEDs" are columns from left to right; they are spread over every device
//! type, so a zone along the bottom of the screen paints the keyboard, the
//! mouse, the mousepad and - through Chroma Link / Chroma Connect - devices
//! of other vendors.

use std::time::Duration;

use serde_json::{json, Value};

use crate::config::DeviceCfg;
use crate::i18n::tr;

pub const PORT: u16 = 54235;
/// columns of the Chroma keyboard grid; the default "LED" count
pub const DEFAULT_COLUMNS: u16 = 22;

/// (REST device, effect, rows, columns)
const DEVICES: &[(&str, &str, usize, usize)] = &[
    ("keyboard", "CHROMA_CUSTOM", 6, 22),
    ("mouse", "CHROMA_CUSTOM2", 9, 7),
    ("mousepad", "CHROMA_CUSTOM", 1, 15),
    ("headset", "CHROMA_CUSTOM", 1, 5),
    ("keypad", "CHROMA_CUSTOM", 4, 5),
    ("chromalink", "CHROMA_CUSTOM", 1, 5),
];

/// Chroma colours are 0x00BBGGRR.
pub fn bgr(c: [u8; 3]) -> u32 {
    ((c[2] as u32) << 16) | ((c[1] as u32) << 8) | c[0] as u32
}

/// Effect body for one device: the columns stretched to its grid, every row
/// the same. One-row devices get a flat array.
pub fn effect(colors: &[[u8; 3]], effect: &str, rows: usize, cols: usize) -> Value {
    let row: Vec<u32> = super::resample(colors, cols).into_iter().map(bgr).collect();
    let param = if rows == 1 {
        json!(row)
    } else {
        json!(vec![row; rows])
    };
    json!({"effect": effect, "param": param})
}

pub fn init_body() -> Value {
    json!({
        "title": "WLED Ambilight",
        "description": "Screen colours on Razer Chroma devices",
        "author": {"name": "Patrick Weppelmann", "contact": "https://github.com/Smotherer007/wledambilight"},
        "device_supported": DEVICES.iter().map(|d| d.0).collect::<Vec<_>>(),
        "category": "application"
    })
}

/// Is Synapse's Chroma SDK server running on this computer?
pub fn available() -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], PORT)),
        Duration::from_millis(300),
    )
    .is_ok()
}

pub struct Driver {
    agent: ureq::Agent,
    uri: Option<String>,
}

impl Driver {
    pub fn new(_dev: &DeviceCfg) -> Self {
        Self {
            agent: super::http_agent(Duration::from_secs(2)),
            uri: None,
        }
    }
}

fn not_running() -> String {
    tr(
        "Razer Synapse nicht erreichbar – läuft Synapse und ist „Chroma Apps“ eingeschaltet?",
        "Razer Synapse not reachable – is Synapse running with “Chroma Apps” switched on?",
    )
    .into()
}

impl super::Driver for Driver {
    fn connect(&mut self) -> Result<(), String> {
        let mut r = self
            .agent
            .post(format!("http://127.0.0.1:{PORT}/razer/chromasdk"))
            .send_json(init_body())
            .map_err(|_| not_running())?;
        let v: Value = r
            .body_mut()
            .read_json()
            .map_err(|e| format!("Razer: {e}"))?;
        let uri = v
            .get("uri")
            .and_then(|u| u.as_str())
            .ok_or_else(not_running)?
            // the server answers with "localhost"; stay on IPv4 like the server
            .replace("localhost", "127.0.0.1");
        self.uri = Some(uri);
        // Synapse needs a moment before a new session accepts effects
        std::thread::sleep(Duration::from_millis(500));
        Ok(())
    }

    fn write(&mut self, colors: &[[u8; 3]]) -> Result<(), String> {
        let uri = self.uri.as_ref().ok_or("not connected")?;
        for (dev, eff, rows, cols) in DEVICES {
            let r = self
                .agent
                .put(format!("{uri}/{dev}"))
                .send_json(effect(colors, eff, *rows, *cols))
                .map_err(|e| format!("Razer: {e}"))?;
            if r.status().is_server_error() || r.status() == 404 {
                return Err(format!("Razer: {} {}", dev, r.status()));
            }
        }
        Ok(())
    }

    fn keepalive(&mut self) -> Result<(), String> {
        let uri = self.uri.as_ref().ok_or("not connected")?;
        self.agent
            .put(format!("{uri}/heartbeat"))
            .send_empty()
            .map(|_| ())
            .map_err(|e| format!("Razer: {e}"))
    }

    fn close(&mut self) {
        if let Some(uri) = self.uri.take() {
            let _ = self.agent.delete(&uri).call();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_order_is_bgr() {
        assert_eq!(bgr([0x11, 0x22, 0x33]), 0x0033_2211);
    }

    #[test]
    fn grids_have_the_right_shape() {
        let cols = vec![[255, 0, 0]; 22];
        let kb = effect(&cols, "CHROMA_CUSTOM", 6, 22);
        assert_eq!(kb["param"].as_array().unwrap().len(), 6);
        assert_eq!(kb["param"][0].as_array().unwrap().len(), 22);
        assert_eq!(kb["param"][0][0], 0xFF);
        let pad = effect(&cols, "CHROMA_CUSTOM", 1, 15);
        assert_eq!(pad["param"].as_array().unwrap().len(), 15);
        let mouse = effect(&cols, "CHROMA_CUSTOM2", 9, 7);
        assert_eq!(mouse["effect"], "CHROMA_CUSTOM2");
        assert_eq!(mouse["param"][8].as_array().unwrap().len(), 7);
    }

    #[test]
    fn init_lists_all_devices() {
        let b = init_body();
        assert_eq!(
            b["device_supported"].as_array().unwrap().len(),
            DEVICES.len()
        );
        assert_eq!(b["category"], "application");
    }
}
