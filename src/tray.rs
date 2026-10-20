//! Tray icon (StatusNotifierItem via ksni): show window, pause, quit.
//! Works with the KDE, GNOME-AppIndicator, Xfce and most other trays.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip, Tray as KsniTray};

#[derive(Clone)]
pub struct TrayState {
    /// user asked for pause (independent of the focus based pause)
    pub manual_pause: Arc<AtomicBool>,
    /// the flag the pipeline actually reads
    pub effective_pause: Arc<AtomicBool>,
    pub show: Arc<AtomicBool>,
    pub quit: Arc<AtomicBool>,
    pub ctx: Arc<Mutex<Option<egui::Context>>>,
    pub status: Arc<Mutex<String>>,
}

impl TrayState {
    pub fn new(effective_pause: Arc<AtomicBool>) -> Self {
        Self {
            manual_pause: Arc::new(AtomicBool::new(false)),
            effective_pause,
            show: Arc::new(AtomicBool::new(false)),
            quit: Arc::new(AtomicBool::new(false)),
            ctx: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new("läuft".to_string())),
        }
    }

    fn wake(&self) {
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }

    fn request_show(&self) {
        self.show.store(true, Ordering::Relaxed);
        self.wake();
    }

    fn toggle_pause(&self) {
        let now = !self.manual_pause.load(Ordering::Relaxed);
        self.manual_pause.store(now, Ordering::Relaxed);
        // while the window is hidden no ui update happens, so set the flag here
        self.effective_pause.store(now, Ordering::Relaxed);
        self.wake();
    }

    fn quit(&self) {
        self.quit.store(true, Ordering::Relaxed);
        self.wake();
        // the event loop may not run a pass while minimised - make sure it ends
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(1200));
            std::process::exit(0);
        });
    }
}

pub struct Tray {
    state: TrayState,
}

fn tray_icon() -> Vec<Icon> {
    let size = 32i32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 / (size - 1) as f32;
            let fy = y as f32 / (size - 1) as f32;
            let dx = fx - 0.5;
            let dy = fy - 0.5;
            let inside = (dx * dx + dy * dy).sqrt() < 0.46;
            if inside {
                data.push(255);
                data.push((70.0 + 150.0 * fy) as u8);
                data.push((120.0 + 60.0 * (1.0 - fx)) as u8);
                data.push(255.0f32.min(200.0 + 60.0 * fx) as u8);
            } else {
                data.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    vec![Icon {
        width: size,
        height: size,
        data,
    }]
}

impl KsniTray for Tray {
    fn id(&self) -> String {
        "ambilight".into()
    }

    fn title(&self) -> String {
        "Ambilight".into()
    }

    fn icon_name(&self) -> String {
        "video-display".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        tray_icon()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Ambilight".into(),
            description: self.state.status.lock().unwrap().clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.state.request_show();
    }

    /// exist so the item is rebuilt every time the menu opens (keeps the
    /// checkbox correct)
    fn menu_about_to_show(&mut self) {}

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::Standard(StandardItem {
                label: "Fenster anzeigen".into(),
                activate: Box::new(|t: &mut Tray| t.state.request_show()),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Checkmark(CheckmarkItem {
                label: "Pause".into(),
                checked: self.state.manual_pause.load(Ordering::Relaxed),
                activate: Box::new(|t: &mut Tray| t.state.toggle_pause()),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Beenden".into(),
                activate: Box::new(|t: &mut Tray| t.state.quit()),
                ..Default::default()
            }),
        ]
    }
}

pub fn spawn(state: TrayState, ctx: egui::Context) -> Option<Handle<Tray>> {
    *state.ctx.lock().unwrap() = Some(ctx);
    match (Tray {
        state: state.clone(),
    })
    .spawn()
    {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("tray unavailable: {e}");
            None
        }
    }
}
