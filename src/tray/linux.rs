// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Linux tray via StatusNotifierItem (`ksni`).

use std::sync::atomic::Ordering;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip};

use super::{TrayState, ICON_32, ICON_64};
use crate::i18n::tr;

struct Item {
    state: TrayState,
}

/// ksni wants ARGB32 in network byte order.
fn argb(rgba: &[u8], size: i32) -> Icon {
    let mut data = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        data.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
    }
    Icon {
        width: size,
        height: size,
        data,
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "wled-ambilight".into()
    }

    fn title(&self) -> String {
        "WLED Ambilight".into()
    }

    fn icon_name(&self) -> String {
        // In a Flatpak the icon is exported to the host theme; elsewhere the
        // pixmap below is the reliable choice (not every host falls back).
        if std::env::var_os("FLATPAK_ID").is_some() {
            crate::APP_ID.into()
        } else {
            String::new()
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![argb(ICON_32, 32), argb(ICON_64, 64)]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "WLED Ambilight".into(),
            description: self.state.status.lock().unwrap().clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.state.request_show();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::Standard(StandardItem {
                label: tr("Fenster anzeigen", "Show window").into(),
                activate: Box::new(|t: &mut Item| t.state.request_show()),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Checkmark(CheckmarkItem {
                label: tr("Pausieren", "Pause").into(),
                checked: self.state.manual_pause.load(Ordering::Relaxed),
                activate: Box::new(|t: &mut Item| t.state.toggle_pause()),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: tr("Beenden", "Quit").into(),
                activate: Box::new(|t: &mut Item| t.state.quit()),
                ..Default::default()
            }),
        ]
    }
}

pub struct Tray {
    handle: Handle<Item>,
}

impl Tray {
    pub fn spawn(state: TrayState) -> Option<Self> {
        // inside a Flatpak we may not own the SNI bus name (see ksni docs)
        let sandboxed = std::env::var_os("FLATPAK_ID").is_some();
        match (Item { state }).disable_dbus_name(sandboxed).spawn() {
            Ok(handle) => Some(Self { handle }),
            Err(e) => {
                eprintln!("tray unavailable: {e}");
                None
            }
        }
    }

    /// Re-read pause state, tooltip and labels.
    pub fn refresh(&self) {
        self.handle.update(|_| {});
    }
}
