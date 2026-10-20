// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Windows notification area / macOS menu bar via `tray-icon`.
//!
//! The icon must be created and kept on the main thread; eframe builds the app
//! there, so `Tray::spawn` is called from `App::new`. Menu clicks arrive on
//! the event loop thread and only flip flags in `TrayState`.

use std::cell::Cell;
use std::sync::atomic::Ordering;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::{TrayState, ICON_32, ICON_64};
use crate::i18n::{self, tr};

const ID_SHOW: &str = "show";
const ID_PAUSE: &str = "pause";
const ID_QUIT: &str = "quit";

pub struct Tray {
    icon: TrayIcon,
    show: MenuItem,
    pause: CheckMenuItem,
    quit: MenuItem,
    state: TrayState,
    tooltip: std::cell::RefCell<String>,
    lang: Cell<i18n::Lang>,
}

impl Tray {
    pub fn spawn(state: TrayState) -> Option<Self> {
        let show = MenuItem::with_id(ID_SHOW, tr("Fenster anzeigen", "Show window"), true, None);
        let pause = CheckMenuItem::with_id(ID_PAUSE, tr("Pausieren", "Pause"), true, false, None);
        let quit = MenuItem::with_id(ID_QUIT, tr("Beenden", "Quit"), true, None);
        let menu = Menu::new();
        let sep1 = PredefinedMenuItem::separator();
        let sep2 = PredefinedMenuItem::separator();
        for item in [
            &show as &dyn tray_icon::menu::IsMenuItem,
            &sep1,
            &pause,
            &sep2,
            &quit,
        ] {
            if let Err(e) = menu.append(item) {
                eprintln!("tray menu: {e}");
            }
        }

        // the macOS menu bar wants a small icon, Windows scales down the bigger one
        let (rgba, size) = if cfg!(target_os = "macos") {
            (ICON_32.to_vec(), 32)
        } else {
            (ICON_64.to_vec(), 64)
        };
        let icon = tray_icon::Icon::from_rgba(rgba, size, size).ok()?;
        let tray = match TrayIconBuilder::new()
            .with_id("wled-ambilight")
            .with_tooltip("WLED Ambilight")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .build()
        {
            Ok(t) => t,
            Err(e) => {
                eprintln!("tray unavailable: {e}");
                return None;
            }
        };

        let s = state.clone();
        MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
            if ev.id == ID_SHOW {
                s.request_show();
            } else if ev.id == ID_PAUSE {
                s.toggle_pause();
            } else if ev.id == ID_QUIT {
                s.quit();
            }
        }));
        let s = state.clone();
        TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
            // Windows: left click on the icon brings the window back
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                s.request_show();
            }
        }));

        Some(Self {
            icon: tray,
            show,
            pause,
            quit,
            state,
            tooltip: std::cell::RefCell::new(String::new()),
            lang: Cell::new(i18n::current()),
        })
    }

    /// Sync checkmark, tooltip and labels with the app state. Cheap; called
    /// from the UI loop.
    pub fn refresh(&self) {
        let paused = self.state.manual_pause.load(Ordering::Relaxed);
        if self.pause.is_checked() != paused {
            self.pause.set_checked(paused);
        }
        let status = self.state.status.lock().unwrap().clone();
        if *self.tooltip.borrow() != status {
            let _ = self
                .icon
                .set_tooltip(Some(format!("WLED Ambilight\n{status}")));
            *self.tooltip.borrow_mut() = status;
        }
        let lang = i18n::current();
        if self.lang.get() != lang {
            self.lang.set(lang);
            self.show.set_text(tr("Fenster anzeigen", "Show window"));
            self.pause.set_text(tr("Pausieren", "Pause"));
            self.quit.set_text(tr("Beenden", "Quit"));
        }
    }
}
