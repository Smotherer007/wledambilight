// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! The window: devices on the left, the screen preview with the zones in the
//! middle, the inspector on the right and the live LED output at the bottom.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, Vec2};

use crate::capture::{self, CaptureStatus};
use crate::config::{Config, DeviceCfg, Protocol, Zone, ZonePreset};
use crate::i18n::{self, tr};
use crate::icons::{self, Icon, Kind};
use crate::theme::{self, HandCursor};
use crate::tray::{Tray, TrayState};
use crate::wled::{self, WledInfo};
use crate::worker::{Pipeline, Shared, Stats};

// ------------------------------------------------------------------ geometry

#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

#[derive(Clone, Copy)]
struct DragState {
    zone: usize,
    grab: Grab,
    start: Pos2,
    orig: (f32, f32, f32, f32),
}

/// Maps normalised zone coordinates to the preview rectangle on screen.
struct View {
    rect: Rect,
}

impl View {
    fn new(avail: Rect, aspect: f32) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            16.0 / 9.0
        };
        let mut w = avail.width();
        let mut h = w / aspect;
        if h > avail.height() {
            h = avail.height();
            w = h * aspect;
        }
        let size = Vec2::new(w.max(32.0), h.max(32.0));
        Self {
            rect: Rect::from_center_size(avail.center(), size),
        }
    }

    fn pos(&self, nx: f32, ny: f32) -> Pos2 {
        Pos2::new(
            self.rect.left() + nx * self.rect.width(),
            self.rect.top() + ny * self.rect.height(),
        )
    }

    fn zone_rect(&self, z: &Zone) -> Rect {
        Rect::from_min_max(self.pos(z.x, z.y), self.pos(z.x + z.w, z.y + z.h))
    }

    fn norm(&self, p: Pos2) -> (f32, f32) {
        (
            (p.x - self.rect.left()) / self.rect.width(),
            (p.y - self.rect.top()) / self.rect.height(),
        )
    }
}

/// A zone counts as sitting on a screen edge within this distance.
const EDGE: f32 = 0.02;

fn preset_name(kind: ZonePreset) -> String {
    match kind {
        ZonePreset::Top => tr("Oben", "Top"),
        ZonePreset::Bottom => tr("Unten", "Bottom"),
        ZonePreset::Left => tr("Links", "Left"),
        ZonePreset::Right => tr("Rechts", "Right"),
        ZonePreset::Full => tr("Vollbild", "Full screen"),
        ZonePreset::Custom => "Zone",
    }
    .to_string()
}

const PALETTE: [[u8; 3]; 6] = [
    [255, 196, 107],
    [255, 94, 143],
    [63, 216, 232],
    [139, 108, 255],
    [91, 227, 166],
    [255, 140, 90],
];

// ------------------------------------------------------------------ state

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Zones,
    Picture,
    App,
}

/// Pipeline values as shown in the window. On Linux they are frozen while the
/// window is unfocused, so the window cannot capture its own changing pixels.
#[derive(Default)]
struct Snapshot {
    last: Vec<Vec<[u8; 3]>>,
    stats: Stats,
}

type Probe = Arc<Mutex<Option<Result<WledInfo, String>>>>;

pub struct App {
    cfg: Config,
    _pipeline: Pipeline,
    shared: Arc<Shared>,
    texture: Option<TextureHandle>,
    texture_seq: u64,
    logo: Option<TextureHandle>,
    drag: Option<DragState>,
    selected: Option<usize>,
    tab: Tab,
    scan: Option<wled::Scan>,
    manual_host: String,
    manual_probe: Option<Probe>,
    expanded: HashSet<String>,
    confirm_remove: Option<String>,
    notice: Option<(String, Instant)>,
    config_dirty: bool,
    last_save: Instant,
    tray: Option<Tray>,
    tray_state: TrayState,
    tray_seen: (String, bool),
    quitting: bool,
    snap: Snapshot,
    monitors: Option<Vec<String>>,
    /// `--minimized`: minimise once the window exists
    start_minimized: bool,
    /// what the pipeline thread currently knows - compared every frame
    pushed: Config,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start_minimized: bool) -> Self {
        theme::install(&cc.egui_ctx);
        let cfg = Config::load();
        i18n::init(&cfg.settings.language);

        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::HasWindowHandle;
            if let Ok(h) = cc.window_handle() {
                capture::exclude_window_from_capture(h.as_raw());
            }
        }

        let pipeline = Pipeline::start(&cfg);
        let shared = pipeline.shared.clone();
        let tray_state = TrayState::new(shared.paused.clone(), cc.egui_ctx.clone());
        let tray = Tray::spawn(tray_state.clone());
        let logo = Some(cc.egui_ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied(
                [64, 64],
                include_bytes!("../assets/icons/app-64.rgba"),
            ),
            egui::TextureOptions::LINEAR,
        ));
        let first_run = cfg.devices.is_empty();
        let mut app = Self {
            _pipeline: pipeline,
            shared,
            texture: None,
            texture_seq: u64::MAX,
            logo,
            drag: None,
            selected: if cfg.zones.is_empty() { None } else { Some(0) },
            tab: Tab::Zones,
            scan: None,
            manual_host: String::new(),
            manual_probe: None,
            expanded: cfg
                .devices
                .first()
                .map(|d| d.id.clone())
                .into_iter()
                .collect(),
            confirm_remove: None,
            notice: None,
            config_dirty: false,
            last_save: Instant::now(),
            tray,
            tray_state,
            tray_seen: (String::new(), false),
            quitting: false,
            snap: Snapshot::default(),
            monitors: None,
            start_minimized,
            pushed: cfg.clone(),
            cfg,
        };
        app.sync_config();
        if first_run {
            app.start_scan();
        }
        app
    }

    // ------------------------------------------------------------ helpers

    /// Push the edited config into the pipeline thread.
    fn sync_config(&mut self) {
        self.shared.set_config(&self.cfg);
        self.pushed = self.cfg.clone();
    }

    fn changed(&mut self) {
        self.sync_config();
        self.config_dirty = true;
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
    }

    fn capture_dims(&self) -> (u32, u32) {
        let d = self.snap.stats.dims;
        if d.0 > 8 && d.1 > 8 {
            d
        } else {
            let w = self.cfg.settings.capture_width.clamp(64, 1280);
            (w, ((w as f32 * 9.0 / 16.0) as u32).max(32))
        }
    }

    fn screen_size(&self) -> (u32, u32) {
        let s = self.snap.stats.screen;
        if s.0 > 0 {
            s
        } else {
            (2560, 1440)
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let f = self.shared.frame.lock().unwrap().clone();
        if f.seq == self.texture_seq && self.texture.is_some() {
            return;
        }
        if f.data.len() != (f.w * f.h * 3) as usize {
            return;
        }
        self.texture_seq = f.seq;
        let img = egui::ColorImage::from_rgb([f.w as usize, f.h as usize], &f.data);
        match self.texture.as_mut() {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => {
                self.texture = Some(ctx.load_texture("preview", img, egui::TextureOptions::LINEAR))
            }
        }
    }

    fn save_if_due(&mut self) {
        if self.config_dirty && self.last_save.elapsed() > Duration::from_millis(400) {
            self.cfg.save();
            self.config_dirty = false;
            self.last_save = Instant::now();
        }
    }

    fn next_zone_color(&self) -> [u8; 3] {
        PALETTE[self.cfg.zones.len() % PALETTE.len()]
    }

    fn start_scan(&mut self) {
        if self.scan.is_none() {
            self.scan = Some(wled::search());
        }
    }

    /// Colours of zone `i` in screen order, as they look (not as wired).
    fn zone_colors(&self, i: usize, ranges: &[(u32, u32)]) -> Vec<Color32> {
        let Some(z) = self.cfg.zones.get(i) else {
            return Vec::new();
        };
        let Some(di) = self.cfg.devices.iter().position(|d| d.id == z.device) else {
            return Vec::new();
        };
        let dev = &self.cfg.devices[di];
        let Some(all) = self.snap.last.get(di) else {
            return Vec::new();
        };
        let n = all.len();
        let (lo, hi) = ranges.get(i).copied().unwrap_or((0, 0));
        (lo..hi)
            .filter_map(|k| {
                let k = k as usize;
                // `last` is in wire order: undo the reversal and the BGR swap
                let idx = if dev.reverse {
                    n.checked_sub(k + 1)?
                } else {
                    k
                };
                let c = all.get(idx)?;
                Some(if dev.bgr {
                    Color32::from_rgb(c[2], c[1], c[0])
                } else {
                    Color32::from_rgb(c[0], c[1], c[2])
                })
            })
            .collect()
    }

    // ------------------------------------------------------------ actions

    /// A new device starts as one zone that uses the whole strip.
    fn add_device(&mut self, host: String, name: String, leds: u16) {
        if self.cfg.device(&host).is_some() {
            self.notify(format!(
                "{host} {}",
                tr("ist schon in der Liste", "is already in the list")
            ));
            return;
        }
        let dev = DeviceCfg::new(&host, name.clone(), leds);
        let id = dev.id.clone();
        let edge = self.next_free_edge();
        let mut zone = Zone::preset(edge, id.clone(), leds.max(1), self.next_zone_color());
        zone.name = name.clone();
        self.cfg.devices.push(dev);
        self.cfg.zones.push(zone);
        self.selected = Some(self.cfg.zones.len() - 1);
        self.expanded.insert(id);
        self.notify(if i18n::is_de() {
            format!("{name} hinzugefügt – eine Zone mit allen {leds} LEDs")
        } else {
            format!("{name} added – one zone with all {leds} LEDs")
        });
        self.changed();
    }

    /// Add another zone on the selected device, using the LEDs that are still free.
    fn add_free_zone(&mut self) {
        let device = self
            .selected
            .and_then(|i| self.cfg.zones.get(i))
            .map(|z| z.device.clone())
            .or_else(|| {
                self.cfg
                    .devices
                    .iter()
                    .find(|d| d.enabled)
                    .map(|d| d.id.clone())
            });
        let Some(device) = device else {
            self.notify(tr(
                "Zuerst ein WLED-Gerät hinzufügen",
                "Add a WLED device first",
            ));
            return;
        };
        let Some(dev) = self.cfg.device(&device).cloned() else {
            return;
        };
        let free = dev
            .led_count
            .saturating_sub(self.cfg.used_leds(&device, None));
        let count = if free > 0 {
            free
        } else {
            (dev.led_count / 4).max(1)
        };
        let mut zone = Zone::preset(ZonePreset::Top, device, count, self.next_zone_color());
        zone.name = format!("Zone {}", self.cfg.zones.len() + 1);
        zone.y = 0.35;
        zone.h = 0.3;
        self.cfg.zones.push(zone);
        self.selected = Some(self.cfg.zones.len() - 1);
        self.changed();
    }

    /// Which screen edge is not taken yet, so that four separate devices end up
    /// around the screen instead of on top of each other.
    fn next_free_edge(&self) -> ZonePreset {
        let order = [
            ZonePreset::Bottom,
            ZonePreset::Top,
            ZonePreset::Left,
            ZonePreset::Right,
        ];
        for kind in order {
            let (x, y, w) = match kind {
                ZonePreset::Top => (0.0, 0.0, 1.0),
                ZonePreset::Bottom => (0.0, 0.88, 1.0),
                ZonePreset::Left => (0.0, 0.0, 0.12),
                _ => (0.88, 0.0, 0.12f32),
            };
            let taken =
                self.cfg.zones.iter().any(|z| {
                    (z.x - x).abs() < 0.2 && (z.y - y).abs() < 0.2 && (z.w - w).abs() < 0.2
                });
            if !taken {
                return kind;
            }
        }
        ZonePreset::Custom
    }

    /// One device per screen edge - the normal case for 2..4 separate WLED boards.
    fn arrange_devices_on_edges(&mut self) {
        let devices: Vec<DeviceCfg> = self
            .cfg
            .devices
            .iter()
            .filter(|d| d.enabled)
            .cloned()
            .collect();
        if devices.is_empty() {
            self.notify(tr("Kein aktives Gerät", "No active device"));
            return;
        }
        let edges: &[ZonePreset] = match devices.len() {
            1 => &[ZonePreset::Bottom],
            2 => &[ZonePreset::Top, ZonePreset::Bottom],
            3 => &[ZonePreset::Top, ZonePreset::Bottom, ZonePreset::Left],
            _ => &[
                ZonePreset::Top,
                ZonePreset::Right,
                ZonePreset::Bottom,
                ZonePreset::Left,
            ],
        };
        let ids: Vec<String> = devices.iter().map(|d| d.id.clone()).collect();
        self.cfg.zones.retain(|z| !ids.contains(&z.device));
        for (i, dev) in devices.iter().enumerate() {
            let kind = edges[i % edges.len()];
            let mut z = Zone::preset(
                kind,
                dev.id.clone(),
                dev.led_count.max(1),
                PALETTE[i % PALETTE.len()],
            );
            z.name = dev.name.clone();
            self.cfg.zones.push(z);
        }
        self.selected = Some(0);
        self.notify(if i18n::is_de() {
            format!(
                "{} Gerät(e) auf die Bildschirmkanten verteilt",
                devices.len()
            )
        } else {
            format!("{} device(s) placed on the screen edges", devices.len())
        });
        self.changed();
    }

    /// Turn the selected zone's device into four edge zones, LED count shared
    /// out by edge length.
    fn split_into_edges(&mut self) {
        let Some(idx) = self.selected else {
            self.notify(tr(
                "Erst die Zone auswählen, die aufgeteilt werden soll",
                "Select the zone to split first",
            ));
            return;
        };
        let Some(zone) = self.cfg.zones.get(idx).cloned() else {
            return;
        };
        let Some(dev) = self.cfg.device(&zone.device).cloned() else {
            return;
        };
        let (cw, ch) = self.capture_dims();
        let counts = split_counts(dev.led_count, ch as f32 / cw as f32);
        // zones of this device are replaced, otherwise the LED ranges would clash
        let first = self
            .cfg
            .zones
            .iter()
            .position(|z| z.device == dev.id)
            .unwrap_or(idx);
        self.cfg.zones.retain(|z| z.device != dev.id);
        // order on the strip: top, right, bottom, left (clockwise)
        let kinds = [
            ZonePreset::Top,
            ZonePreset::Right,
            ZonePreset::Bottom,
            ZonePreset::Left,
        ];
        let new_zones: Vec<Zone> = kinds
            .iter()
            .enumerate()
            .map(|(i, kind)| {
                let mut z = Zone::preset(*kind, dev.id.clone(), counts[i], PALETTE[i]);
                z.name = preset_name(*kind);
                z
            })
            .collect();
        let at = first.min(self.cfg.zones.len());
        self.cfg.zones.splice(at..at, new_zones);
        self.selected = Some(at);
        self.notify(if i18n::is_de() {
            format!(
                "{} in 4 Kanten aufgeteilt: {} / {} / {} / {} LEDs",
                dev.name, counts[0], counts[1], counts[2], counts[3]
            )
        } else {
            format!(
                "{} split into 4 edges: {} / {} / {} / {} LEDs",
                dev.name, counts[0], counts[1], counts[2], counts[3]
            )
        });
        self.changed();
    }

    fn remove_zone(&mut self, i: usize) {
        if i < self.cfg.zones.len() {
            self.cfg.zones.remove(i);
            self.selected = None;
            self.changed();
        }
    }

    fn apply_drag(&mut self, view: &View, drag: &DragState, pos: Pos2) {
        let (nx, ny) = view.norm(pos);
        let (nx, ny) = (nx.clamp(-0.5, 1.5), ny.clamp(-0.5, 1.5));
        let (sx, sy) = view.norm(drag.start);
        let (dx, dy) = (nx - sx, ny - sy);
        let (ox, oy, ow, oh) = drag.orig;
        let (mut x, mut y, mut w, mut h) = (ox, oy, ow, oh);
        let west = matches!(drag.grab, Grab::W | Grab::NW | Grab::SW);
        let east = matches!(drag.grab, Grab::E | Grab::NE | Grab::SE);
        let north = matches!(drag.grab, Grab::N | Grab::NW | Grab::NE);
        let south = matches!(drag.grab, Grab::S | Grab::SW | Grab::SE);
        if drag.grab == Grab::Move {
            x = ox + dx;
            y = oy + dy;
        }
        if west {
            x = ox + dx;
            w = ow - dx;
        }
        if east {
            w = ow + dx;
        }
        if north {
            y = oy + dy;
            h = oh - dy;
        }
        if south {
            h = oh + dy;
        }
        // one capture pixel is the smallest unit - at 320x180 that is ~8 screen
        // pixels, at 1280x720 it is ~2. Raise the capture width for finer control.
        let (cap_w, cap_h) = self.capture_dims();
        let min_w = 1.0 / cap_w.max(1) as f32;
        let min_h = 1.0 / cap_h.max(1) as f32;
        if w < min_w {
            if west {
                x = ox + ow - min_w;
            }
            w = min_w;
        }
        if h < min_h {
            if north {
                y = oy + oh - min_h;
            }
            h = min_h;
        }
        if self.cfg.settings.snap {
            let t = 7.0 / view.rect.width().max(1.0);
            let ty = 7.0 / view.rect.height().max(1.0);
            let mut xs = vec![0.0, 0.5, 1.0];
            let mut ys = vec![0.0, 0.5, 1.0];
            for (i, z) in self.cfg.zones.iter().enumerate() {
                if i != drag.zone {
                    xs.extend([z.x, z.x + z.w]);
                    ys.extend([z.y, z.y + z.h]);
                }
            }
            let near =
                |vals: &[f32], v: f32, tol: f32| vals.iter().copied().find(|s| (v - s).abs() < tol);
            if drag.grab == Grab::Move {
                if let Some(v) = near(&xs, x, t) {
                    x = v;
                } else if let Some(v) = near(&xs, x + w, t) {
                    x = v - w;
                }
                if let Some(v) = near(&ys, y, ty) {
                    y = v;
                } else if let Some(v) = near(&ys, y + h, ty) {
                    y = v - h;
                }
            } else {
                if west {
                    if let Some(v) = near(&xs, x, t) {
                        w = x + w - v;
                        x = v;
                    }
                }
                if east {
                    if let Some(v) = near(&xs, x + w, t) {
                        w = v - x;
                    }
                }
                if north {
                    if let Some(v) = near(&ys, y, ty) {
                        h = y + h - v;
                        y = v;
                    }
                }
                if south {
                    if let Some(v) = near(&ys, y + h, ty) {
                        h = v - y;
                    }
                }
            }
        }
        x = x.clamp(-w + min_w, 1.0 - min_w);
        y = y.clamp(-h + min_h, 1.0 - min_h);
        w = w.clamp(min_w, 1.0 - x.max(0.0));
        h = h.clamp(min_h, 1.0 - y.max(0.0));
        if let Some(z) = self.cfg.zones.get_mut(drag.zone) {
            z.x = x;
            z.y = y;
            z.w = w;
            z.h = h;
        }
        self.changed();
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        // never steal keys from text fields
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let Some(i) = self.selected.filter(|i| *i < self.cfg.zones.len()) else {
            return;
        };
        let (mut dx, mut dy) = (0.0f32, 0.0f32);
        let mut delete = false;
        let mut deselect = false;
        ctx.input(|inp| {
            let step = if inp.modifiers.shift { 0.02 } else { 0.002 };
            if inp.key_pressed(egui::Key::ArrowLeft) {
                dx -= step;
            }
            if inp.key_pressed(egui::Key::ArrowRight) {
                dx += step;
            }
            if inp.key_pressed(egui::Key::ArrowUp) {
                dy -= step;
            }
            if inp.key_pressed(egui::Key::ArrowDown) {
                dy += step;
            }
            delete = inp.key_pressed(egui::Key::Delete);
            deselect = inp.key_pressed(egui::Key::Escape);
        });
        if delete {
            self.remove_zone(i);
            return;
        }
        if deselect {
            self.selected = None;
            return;
        }
        if dx != 0.0 || dy != 0.0 {
            if let Some(z) = self.cfg.zones.get_mut(i) {
                z.x = (z.x + dx).clamp(-z.w + 0.01, 1.0 - 0.01);
                z.y = (z.y + dy).clamp(-z.h + 0.01, 1.0 - 0.01);
            }
            self.changed();
        }
    }
}

/// Share `total` LEDs over top, right, bottom, left by edge length.
fn split_counts(total: u16, ratio: f32) -> [u16; 4] {
    let lens = [1.0f32, ratio, 1.0, ratio];
    let sum: f32 = lens.iter().sum();
    let t = total as f32;
    let mut counts = lens.map(|l| (l / sum * t).floor().max(1.0) as u16);
    let mut diff = total as i32 - counts.iter().map(|c| *c as i32).sum::<i32>();
    while diff > 0 {
        let best = (0..4)
            .max_by(|a, b| {
                let ra = lens[*a] / sum * t - counts[*a] as f32;
                let rb = lens[*b] / sum * t - counts[*b] as f32;
                ra.total_cmp(&rb)
            })
            .unwrap_or(0);
        counts[best] += 1;
        diff -= 1;
    }
    while diff < 0 {
        let Some(i) = (0..4).max_by_key(|i| counts[*i]) else {
            break;
        };
        if counts[i] <= 1 {
            break;
        }
        counts[i] -= 1;
        diff += 1;
    }
    counts
}

// ------------------------------------------------------------------ canvas helpers

/// One cell per LED, filled with the colour that is currently being sent.
fn draw_ruler(
    painter: &egui::Painter,
    rect: Rect,
    colors: &[Color32],
    vertical: bool,
    opacity: f32,
) {
    painter.rect_filled(
        rect.expand(1.5),
        CornerRadius::same(3),
        Color32::from_black_alpha(200),
    );
    let n = colors.len() as f32;
    for (i, c) in colors.iter().enumerate() {
        let a = i as f32 / n;
        let b = (i + 1) as f32 / n;
        let cell = if vertical {
            let y0 = rect.top() + a * rect.height();
            let y1 = rect.top() + b * rect.height();
            Rect::from_min_max(
                Pos2::new(rect.left(), y0),
                Pos2::new(rect.right(), (y1 - 0.6).max(y0 + 0.4)),
            )
        } else {
            let x0 = rect.left() + a * rect.width();
            let x1 = rect.left() + b * rect.width();
            Rect::from_min_max(
                Pos2::new(x0, rect.top()),
                Pos2::new((x1 - 0.6).max(x0 + 0.4), rect.bottom()),
            )
        };
        painter.rect_filled(cell, 0, c.gamma_multiply(opacity));
    }
}

fn hit(r: Rect, p: Pos2) -> Option<Grab> {
    if !r.expand(5.0).contains(p) {
        return None;
    }
    if r.width() < 8.0 || r.height() < 8.0 {
        return Some(Grab::Move);
    }
    let m = 9.0;
    let near = |a: f32, b: f32| (a - b).abs() < m;
    let (l, rr, t, b) = (
        near(p.x, r.left()),
        near(p.x, r.right()),
        near(p.y, r.top()),
        near(p.y, r.bottom()),
    );
    Some(match (l, rr, t, b) {
        (true, _, true, _) => Grab::NW,
        (_, true, true, _) => Grab::NE,
        (true, _, _, true) => Grab::SW,
        (_, true, _, true) => Grab::SE,
        (true, _, _, _) => Grab::W,
        (_, true, _, _) => Grab::E,
        (_, _, true, _) => Grab::N,
        (_, _, _, true) => Grab::S,
        _ => Grab::Move,
    })
}

fn snap_to_edge(z: &mut Zone) {
    let cx = z.x + z.w / 2.0;
    let cy = z.y + z.h / 2.0;
    let (dl, dr, dt, db) = (cx, 1.0 - cx, cy, 1.0 - cy);
    let min = dl.min(dr).min(dt).min(db);
    if min == dl {
        z.x = 0.0;
    } else if min == dr {
        z.x = 1.0 - z.w;
    } else if min == dt {
        z.y = 0.0;
    } else {
        z.y = 1.0 - z.h;
    }
}

fn status_text(s: &CaptureStatus) -> (String, Color32) {
    match s {
        CaptureStatus::Running { .. } => (tr("Live", "Live").to_string(), theme::MINT),
        CaptureStatus::Starting => (
            tr("Aufnahme startet …", "Starting capture …").to_string(),
            theme::AMBER,
        ),
        CaptureStatus::Error(_) => (
            tr("Aufnahme gestoppt", "Capture stopped").to_string(),
            theme::CORAL,
        ),
        CaptureStatus::Idle => (
            tr("Wartet auf Aufnahme", "Waiting for capture").to_string(),
            theme::INK_FAINT,
        ),
    }
}

// ------------------------------------------------------------------ panels

impl App {
    fn top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("top")
            .exact_size(56.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::WALL)
                    .inner_margin(egui::Margin::symmetric(18, 0))
                    .stroke(Stroke::new(1.0_f32, theme::LINE)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(logo) = self.logo.as_ref() {
                        ui.add(egui::Image::new((logo.id(), Vec2::splat(26.0))));
                    }
                    ui.add_space(2.0);
                    ui.label(RichText::new("WLED Ambilight").font(theme::semibold(15.0)));
                    ui.label(
                        RichText::new(format!("v{}", crate::VERSION))
                            .size(11.5)
                            .color(theme::INK_FAINT),
                    );
                    ui.add_space(14.0);

                    let paused = self.shared.paused.load(Ordering::Relaxed);
                    let stats = &self.snap.stats;
                    let (text, color) = if paused {
                        (tr("Pausiert", "Paused").to_string(), theme::AMBER)
                    } else {
                        let (t, c) = status_text(&stats.status);
                        if matches!(stats.status, CaptureStatus::Running { .. })
                            && stats.fps_out > 0.5
                        {
                            (format!("{t} · {:.0} fps", stats.fps_out), c)
                        } else {
                            (t, c)
                        }
                    };
                    theme::status_pill(ui, &text, color);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let manual = self.tray_state.manual_pause.load(Ordering::Relaxed);
                        let (icon, label) = if manual {
                            (Icon::Play, tr("Fortsetzen", "Resume"))
                        } else {
                            (Icon::Pause, tr("Pausieren", "Pause"))
                        };
                        if icons::labelled(ui, icon, label, Kind::Secondary) {
                            self.tray_state
                                .manual_pause
                                .store(!manual, Ordering::Relaxed);
                        }
                        if icons::button(
                            ui,
                            Icon::Refresh,
                            tr("Aufnahme neu starten", "Restart capture"),
                        ) {
                            self.shared.request_restart();
                        }
                    });
                });
            });
    }

    fn devices_panel(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("devices")
            .resizable(true)
            .default_size(284.0)
            .size_range(250.0..=440.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin {
                        left: 16,
                        right: 14,
                        top: 16,
                        bottom: 12,
                    }),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    theme::title(ui, tr("Geräte", "Devices"));
                    if !self.cfg.devices.is_empty() {
                        ui.label(
                            RichText::new(self.cfg.devices.len().to_string())
                                .color(theme::INK_FAINT),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.scan.is_some() {
                            ui.add(egui::Spinner::new().size(16.0).color(theme::INK_DIM));
                        } else if icons::button(
                            ui,
                            Icon::Search,
                            tr("Netzwerk durchsuchen", "Search the network"),
                        ) {
                            self.start_scan();
                        }
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.cfg.devices.is_empty() {
                            self.empty_devices(ui);
                        }
                        self.device_cards(ui);
                        self.discovery(ui);
                        ui.add_space(10.0);
                        self.manual_add(ui);
                    });
            });
    }

    fn empty_devices(&mut self, ui: &mut egui::Ui) {
        theme::card_with(ui, theme::SURFACE, theme::LINE, |ui| {
            ui.label(
                RichText::new(tr("Noch kein Gerät", "No device yet")).font(theme::semibold(13.5)),
            );
            ui.label(
                RichText::new(tr(
                    "WLED-Geräte im selben Netzwerk werden automatisch gefunden. Ein Klick auf + fügt sie hinzu.",
                    "WLED devices on the same network are found automatically. Click + to add one.",
                ))
                .size(12.0)
                .color(theme::INK_DIM),
            );
            if self.scan.is_none() {
                ui.add_space(4.0);
                if icons::labelled(
                    ui,
                    Icon::Search,
                    tr("Netzwerk durchsuchen", "Search the network"),
                    Kind::Primary,
                ) {
                    self.start_scan();
                }
            }
        });
        ui.add_space(8.0);
    }

    fn device_cards(&mut self, ui: &mut egui::Ui) {
        let mut devices = std::mem::take(&mut self.cfg.devices);
        let mut remove: Option<usize> = None;
        let mut any_change = false;
        for (i, dev) in devices.iter_mut().enumerate() {
            let used = self.cfg.used_leds(&dev.id, None);
            let open = self.expanded.contains(&dev.id);
            let before = dev.clone();
            theme::card_with(
                ui,
                theme::SURFACE,
                if open { theme::LINE_HI } else { theme::LINE },
                |ui| {
                    // header: always visible
                    let header = ui.horizontal(|ui| {
                        let dot = if dev.enabled {
                            theme::MINT
                        } else {
                            theme::INK_FAINT
                        };
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, dot);
                        ui.label(RichText::new(&dev.name).font(theme::semibold(13.5)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            theme::switch(ui, &mut dev.enabled, "")
                                .on_hover_text(tr("Gerät aktiv", "Device active"));
                            let chev = if open {
                                Icon::ChevronUp
                            } else {
                                Icon::ChevronDown
                            };
                            if icons::button(ui, chev, tr("Einstellungen", "Settings")) {
                                if open {
                                    self.expanded.remove(&dev.id);
                                } else {
                                    self.expanded.insert(dev.id.clone());
                                }
                            }
                        });
                    });
                    let _ = header;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&dev.host).size(11.5).color(theme::INK_FAINT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let txt = if i18n::is_de() {
                                format!("{used} / {} LEDs belegt", dev.led_count)
                            } else {
                                format!("{used} / {} LEDs used", dev.led_count)
                            };
                            ui.label(RichText::new(txt).size(11.5).color(
                                if used > dev.led_count {
                                    theme::CORAL
                                } else {
                                    theme::INK_DIM
                                },
                            ));
                        });
                    });
                    theme::usage_bar(ui, used as f32, dev.led_count as f32);
                    if used > dev.led_count {
                        theme::hint(
                            ui,
                            tr(
                                "Die Zonen brauchen mehr LEDs, als der Streifen hat.",
                                "The zones need more LEDs than the strip has.",
                            ),
                        );
                    }

                    if !open {
                        return;
                    }
                    ui.add_space(8.0);
                    theme::field_label(ui, tr("Protokoll", "Protocol"));
                    theme::segmented(
                        ui,
                        &mut dev.protocol,
                        &[
                            (Protocol::Ddp, "DDP · 4048"),
                            (Protocol::Drgb, "DRGB · 21324"),
                        ],
                        Some(ui.available_width()),
                    );
                    if dev.protocol == Protocol::Drgb && dev.led_count as usize > wled::DRGB_MAX {
                        theme::hint(
                            ui,
                            tr(
                                "DRGB überträgt höchstens 490 LEDs – für mehr DDP nutzen.",
                                "DRGB carries at most 490 LEDs – use DDP for more.",
                            ),
                        );
                    }
                    ui.add_space(4.0);
                    theme::slider_row(
                        ui,
                        tr("Helligkeit", "Brightness"),
                        &mut dev.brightness,
                        0.0..=1.0,
                        |v| format!("{:.0} %", v * 100.0),
                    );
                    ui.horizontal(|ui| {
                        theme::field_label(ui, tr("Max. Bildrate", "Max. frame rate"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let opts = [0u32, 15, 20, 24, 25, 30, 40, 50, 60, 72, 100, 120, 144];
                            theme::combo(
                                ui,
                                ("fps", i),
                                96.0,
                                &mut dev.max_fps,
                                |v| {
                                    if v == 0 {
                                        tr("global", "global").to_string()
                                    } else {
                                        format!("{v} fps")
                                    }
                                },
                                &opts,
                            );
                        });
                    });
                    ui.add_space(2.0);
                    theme::switch(
                        ui,
                        &mut dev.reverse,
                        tr("Streifen läuft rückwärts", "Strip runs backwards"),
                    )
                    .on_hover_text(tr(
                        "Spiegelt den ganzen Streifen, wenn die Datenleitung am anderen Ende sitzt",
                        "Mirrors the whole strip when the data line is at the other end",
                    ));
                    theme::switch(
                        ui,
                        &mut dev.bgr,
                        tr("Rot und Blau tauschen (BGR)", "Swap red and blue (BGR)"),
                    )
                    .on_hover_text(tr(
                        "Nur für Streifen mit vertauschten Kanälen",
                        "Only for strips with swapped channels",
                    ));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if icons::labelled(
                            ui,
                            Icon::Power,
                            tr("LEDs aus", "LEDs off"),
                            Kind::Secondary,
                        ) {
                            let mut sender = wled::Sender::new(&dev.host, dev.protocol);
                            sender.send(&vec![[0u8; 3]; dev.led_count as usize]);
                        }
                        if self.confirm_remove.as_deref() == Some(dev.id.as_str()) {
                            if icons::labelled(
                                ui,
                                Icon::Trash,
                                tr("Wirklich entfernen", "Really remove"),
                                Kind::Primary,
                            ) {
                                remove = Some(i);
                                self.confirm_remove = None;
                            }
                        } else if icons::labelled(
                            ui,
                            Icon::Trash,
                            tr("Entfernen", "Remove"),
                            Kind::Ghost,
                        ) {
                            self.confirm_remove = Some(dev.id.clone());
                        }
                    });
                },
            );
            if *dev != before {
                any_change = true;
            }
            ui.add_space(8.0);
        }
        if let Some(i) = remove {
            let id = devices[i].id.clone();
            devices.remove(i);
            self.cfg.zones.retain(|z| z.device != id);
            self.selected = None;
            any_change = true;
        }
        self.cfg.devices = devices;
        if any_change {
            self.changed();
        }
    }

    fn discovery(&mut self, ui: &mut egui::Ui) {
        let Some(scan) = self.scan.as_ref() else {
            return;
        };
        let list = scan.found.lock().unwrap().clone();
        let finished = *scan.done.lock().unwrap();
        let pending: Vec<WledInfo> = list
            .into_iter()
            .filter(|i| self.cfg.device(&i.host).is_none())
            .collect();
        if finished && pending.is_empty() {
            let found_any = !self.cfg.devices.is_empty();
            self.scan = None;
            if !found_any {
                self.notify(tr(
                    "Kein WLED-Gerät gefunden. Gleiches Netzwerk? Sonst per Adresse hinzufügen.",
                    "No WLED device found. Same network? Otherwise add it by address.",
                ));
            }
            return;
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            theme::field_label(ui, tr("Im Netzwerk gefunden", "Found on the network"));
            if !finished {
                ui.add(egui::Spinner::new().size(12.0).color(theme::INK_FAINT));
            }
        });
        for info in &pending {
            theme::card_with(ui, theme::WALL, theme::LINE, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(&info.name).font(theme::semibold(13.0)));
                        ui.label(
                            RichText::new(format!(
                                "{} · {} LEDs · v{} · {}",
                                info.host, info.leds, info.version, info.arch
                            ))
                            .size(11.5)
                            .color(theme::INK_FAINT),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Icon::Plus, tr("Hinzufügen", "Add")) {
                            self.add_device(info.host.clone(), info.name.clone(), info.leds);
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
        if !finished {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }

    fn manual_add(&mut self, ui: &mut egui::Ui) {
        theme::field_label(ui, tr("Per Adresse hinzufügen", "Add by address"));
        let mut submit = false;
        ui.horizontal(|ui| {
            // text field + its inner margin + spacing + the 28 px button
            let w = (ui.available_width() - 28.0 - 8.0 - 16.0).max(60.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.manual_host)
                    .hint_text(RichText::new("192.168.1.50").color(theme::INK_FAINT))
                    .desired_width(w)
                    .margin(egui::Margin::symmetric(8, 6)),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }
            if icons::button(
                ui,
                Icon::Plus,
                tr("Gerät prüfen und hinzufügen", "Check and add device"),
            ) {
                submit = true;
            }
        });
        if submit && !self.manual_host.trim().is_empty() && self.manual_probe.is_none() {
            self.manual_probe = Some(wled::probe_in_background(self.manual_host.clone()));
        }
        if let Some(p) = self.manual_probe.clone() {
            let res = p.lock().unwrap().take();
            match res {
                None => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        theme::hint(ui, tr("Verbinde …", "Connecting …"));
                    });
                    ui.ctx().request_repaint_after(Duration::from_millis(150));
                }
                Some(Ok(info)) => {
                    self.manual_probe = None;
                    self.manual_host.clear();
                    self.add_device(info.host, info.name, info.leds);
                }
                Some(Err(host)) => {
                    self.manual_probe = None;
                    self.notify(if i18n::is_de() {
                        format!("Unter {host} antwortet kein WLED-Gerät")
                    } else {
                        format!("No WLED device answers at {host}")
                    });
                }
            }
        }
    }

    fn inspector(&mut self, root: &mut egui::Ui) {
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(312.0)
            .size_range(280.0..=460.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 16,
                        top: 14,
                        bottom: 12,
                    }),
            )
            .show(root, |ui| {
                let w = ui.available_width();
                theme::segmented(
                    ui,
                    &mut self.tab,
                    &[
                        (Tab::Zones, tr("Zonen", "Zones")),
                        (Tab::Picture, tr("Bild", "Picture")),
                        (Tab::App, tr("App", "App")),
                    ],
                    Some(w),
                );
                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tab {
                        Tab::Zones => self.zones_tab(ui),
                        Tab::Picture => self.picture_tab(ui),
                        Tab::App => self.app_tab(ui),
                    });
            });
    }

    fn zones_tab(&mut self, ui: &mut egui::Ui) {
        if self.cfg.zones.is_empty() {
            theme::card(ui, |ui| {
                ui.label(
                    RichText::new(tr("Noch keine Zonen", "No zones yet"))
                        .font(theme::semibold(13.5)),
                );
                ui.label(
                    RichText::new(tr(
                        "Eine Zone ist ein Bildbereich, dessen Farbe auf einen Abschnitt des Streifens geht. Neue Geräte bekommen automatisch eine Zone.",
                        "A zone is an area of the picture whose colour drives a section of the strip. New devices get one automatically.",
                    ))
                    .size(12.0)
                    .color(theme::INK_DIM),
                );
            });
            ui.add_space(8.0);
        } else {
            theme::hint(
                ui,
                tr(
                    "Die Reihenfolge der Liste ist die Reihenfolge auf dem Streifen.",
                    "The order of this list is the order on the strip.",
                ),
            );
            ui.add_space(4.0);
        }

        let ranges = self.cfg.zone_ranges();
        let mut moved: Option<(usize, isize)> = None;
        let mut remove: Option<usize> = None;
        for i in 0..self.cfg.zones.len() {
            let z = &self.cfg.zones[i];
            let selected = self.selected == Some(i);
            let color = Color32::from_rgb(z.color[0], z.color[1], z.color[2]);
            let (lo, hi) = ranges.get(i).copied().unwrap_or((0, 0));
            let name = z.name.clone();
            let enabled = z.enabled;
            let dev_name = self
                .cfg
                .device(&z.device)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| "—".into());
            let resp = egui::Frame::new()
                .fill(if selected {
                    theme::RAISED
                } else {
                    theme::PANEL
                })
                .stroke(Stroke::new(
                    1.0_f32,
                    if selected {
                        theme::LINE_HI
                    } else {
                        Color32::TRANSPARENT
                    },
                ))
                .corner_radius(CornerRadius::same(theme::R_MD))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 30.0), Sense::hover());
                        ui.painter().rect_filled(
                            r,
                            CornerRadius::same(2),
                            if enabled { color } else { theme::LINE_HI },
                        );
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(RichText::new(&name).font(theme::semibold(13.0)).color(
                                if enabled {
                                    theme::INK
                                } else {
                                    theme::INK_FAINT
                                },
                            ));
                            ui.label(
                                RichText::new(format!(
                                    "LED {}–{} · {dev_name}",
                                    lo,
                                    hi.saturating_sub(1)
                                ))
                                .size(11.5)
                                .color(theme::INK_FAINT),
                            );
                        });
                        if selected {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if icons::button_tinted(
                                        ui,
                                        Icon::Trash,
                                        tr("Zone löschen (Entf)", "Delete zone (Del)"),
                                        Some(theme::CORAL),
                                    ) {
                                        remove = Some(i);
                                    }
                                    if icons::button(
                                        ui,
                                        Icon::ChevronDown,
                                        tr("Nach hinten", "Move down"),
                                    ) {
                                        moved = Some((i, 1));
                                    }
                                    if icons::button(
                                        ui,
                                        Icon::ChevronUp,
                                        tr("Nach vorne", "Move up"),
                                    ) {
                                        moved = Some((i, -1));
                                    }
                                },
                            );
                        }
                    });
                })
                .response;
            let click = ui.interact(
                resp.rect,
                ui.scope_id().with(("zone-row", i)),
                Sense::click(),
            );
            if click.clicked() {
                self.selected = Some(i);
            }
            if !selected {
                click.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
            ui.add_space(2.0);
        }
        if let Some((i, d)) = moved {
            let j = (i as isize + d).clamp(0, self.cfg.zones.len() as isize - 1) as usize;
            if j != i {
                self.cfg.zones.swap(i, j);
                self.selected = Some(j);
                self.changed();
            }
        }
        if let Some(i) = remove {
            self.remove_zone(i);
        }

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if icons::labelled(ui, Icon::Plus, tr("Zone", "Zone"), Kind::Primary) {
                self.add_free_zone();
            }
            if icons::labelled_response(
                ui,
                Icon::Split,
                tr("Auf 4 Kanten", "Split to 4 edges"),
                Kind::Secondary,
            )
            .on_hover_text(tr(
                "Teilt den Streifen der ausgewählten Zone auf alle vier Bildschirmkanten auf",
                "Spreads the strip of the selected zone over all four screen edges",
            ))
            .clicked()
            {
                self.split_into_edges();
            }
            if icons::labelled_response(
                ui,
                Icon::Grid,
                tr("Gerät pro Kante", "Device per edge"),
                Kind::Secondary,
            )
            .on_hover_text(tr(
                "Legt jedes aktive Gerät auf eine eigene Bildschirmkante",
                "Puts every active device on its own screen edge",
            ))
            .clicked()
            {
                self.arrange_devices_on_edges();
            }
        });

        if let Some(i) = self.selected.filter(|i| *i < self.cfg.zones.len()) {
            ui.add_space(14.0);
            theme::title(ui, tr("Ausgewählte Zone", "Selected zone"));
            ui.add_space(4.0);
            self.zone_editor(ui, i);
        }
    }

    fn zone_editor(&mut self, ui: &mut egui::Ui, i: usize) {
        let devices: Vec<(String, String)> = self
            .cfg
            .devices
            .iter()
            .map(|d| (d.id.clone(), d.name.clone()))
            .collect();
        let cap = self.capture_dims();
        let (sw, sh) = self.screen_size();
        let reversed = self
            .cfg
            .device(&self.cfg.zones[i].device)
            .map(|d| d.reverse)
            .unwrap_or(false);
        let before = self.cfg.zones[i].clone();
        let z = &mut self.cfg.zones[i];
        theme::card(ui, |ui| {
            theme::field_label(ui, tr("Name", "Name"));
            ui.add(
                egui::TextEdit::singleline(&mut z.name)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(8, 6)),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                theme::field_label(ui, tr("Gerät", "Device"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("zone-device")
                        .width(150.0)
                        .selected_text(
                            devices
                                .iter()
                                .find(|(id, _)| *id == z.device)
                                .map(|(_, n)| n.clone())
                                .unwrap_or_else(|| "—".into()),
                        )
                        .show_ui(ui, |ui| {
                            for (id, name) in &devices {
                                ui.selectable_value(&mut z.device, id.clone(), name.clone())
                                    .hand_cursor();
                            }
                        })
                        .response
                        .hand_cursor();
                });
            });
            ui.horizontal(|ui| {
                theme::field_label(ui, tr("Anzahl LEDs", "LED count"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut c = z.count as u32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut c)
                                .speed(1.0)
                                .range(1..=4000)
                                .suffix(" LEDs"),
                        )
                        .changed()
                    {
                        z.count = c as u16;
                    }
                });
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                theme::switch(ui, &mut z.enabled, tr("Aktiv", "Active"));
                ui.add_space(8.0);
                theme::switch(ui, &mut z.vertical, tr("Senkrecht", "Vertical")).on_hover_text(tr(
                    "LEDs laufen von oben nach unten statt von links nach rechts",
                    "LEDs run top to bottom instead of left to right",
                ));
            });
            theme::hint(
                ui,
                if reversed {
                    tr(
                        "Das Gerät läuft rückwärts – LED 0 liegt am anderen Ende.",
                        "The device runs backwards – LED 0 is at the other end.",
                    )
                } else {
                    tr(
                        "Die Laufrichtung wird pro Gerät eingestellt.",
                        "The direction is set per device.",
                    )
                },
            );
            ui.add_space(6.0);
            theme::field_label(ui, tr("Größe im Aufnahmebild", "Size in the capture"));
            ui.horizontal(|ui| {
                let mut pw = (z.w * cap.0 as f32).max(1.0);
                let mut ph = (z.h * cap.1 as f32).max(1.0);
                if ui
                    .add(
                        egui::DragValue::new(&mut pw)
                            .speed(1.0)
                            .range(1.0..=cap.0 as f32)
                            .suffix(" px"),
                    )
                    .on_hover_text(tr("Breite", "Width"))
                    .changed()
                {
                    z.w = (pw / cap.0 as f32).clamp(1.0 / cap.0 as f32, 1.0);
                }
                ui.label(RichText::new("×").color(theme::INK_FAINT));
                if ui
                    .add(
                        egui::DragValue::new(&mut ph)
                            .speed(1.0)
                            .range(1.0..=cap.1 as f32)
                            .suffix(" px"),
                    )
                    .on_hover_text(tr("Höhe", "Height"))
                    .changed()
                {
                    z.h = (ph / cap.1 as f32).clamp(1.0 / cap.1 as f32, 1.0);
                }
            });
            theme::hint(
                ui,
                &if i18n::is_de() {
                    format!(
                        "≈ {:.0} × {:.0} Bildschirmpixel bei {sw}×{sh}",
                        z.w * sw as f32,
                        z.h * sh as f32
                    )
                } else {
                    format!(
                        "≈ {:.0} × {:.0} screen pixels at {sw}×{sh}",
                        z.w * sw as f32,
                        z.h * sh as f32
                    )
                },
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(tr("Volle Breite", "Full width"))
                    .hand_cursor()
                    .clicked()
                {
                    z.x = 0.0;
                    z.w = 1.0;
                }
                if ui
                    .button(tr("Volle Höhe", "Full height"))
                    .hand_cursor()
                    .clicked()
                {
                    z.y = 0.0;
                    z.h = 1.0;
                }
                if ui.button(tr("An Kante", "To edge")).hand_cursor().clicked() {
                    snap_to_edge(z);
                }
            });
            ui.add_space(6.0);
            theme::slider_row(
                ui,
                tr("Verstärkung", "Gain"),
                &mut z.gain,
                0.0..=3.0,
                |v| format!("{v:.2}×"),
            );
        });
        if self.cfg.zones[i] != before {
            self.changed();
        }
    }

    fn picture_tab(&mut self, ui: &mut egui::Ui) {
        let before = self.cfg.settings.clone();
        let st = &mut self.cfg.settings;
        theme::title(ui, tr("Farbe", "Colour"));
        ui.add_space(4.0);
        theme::card(ui, |ui| {
            theme::slider_row(
                ui,
                tr("Helligkeit", "Brightness"),
                &mut st.brightness,
                0.0..=1.0,
                |v| format!("{:.0} %", v * 100.0),
            );
            theme::slider_row(
                ui,
                tr("Sättigung", "Saturation"),
                &mut st.saturation,
                0.0..=2.0,
                |v| format!("{:.0} %", v * 100.0),
            );
            theme::slider_row(ui, "Gamma", &mut st.gamma, 0.5..=2.5, |v| format!("{v:.2}"));
        });
        ui.add_space(12.0);
        theme::title(ui, tr("Bewegung", "Motion"));
        ui.add_space(4.0);
        theme::card(ui, |ui| {
            theme::slider_row(
                ui,
                tr("Zeitliche Glättung", "Temporal smoothing"),
                &mut st.smoothing_ms,
                0.0..=600.0,
                |v| {
                    if v <= 0.0 {
                        tr("aus", "off").to_string()
                    } else {
                        format!("{v:.0} ms")
                    }
                },
            );
            theme::slider_row(
                ui,
                tr("Räumliche Glättung", "Spatial smoothing"),
                &mut st.spatial,
                0.0..=1.0,
                |v| format!("{:.0} %", v * 100.0),
            );
            theme::slider_row(
                ui,
                tr("Max. Änderung pro Sekunde", "Max. change per second"),
                &mut st.slew,
                0.0..=3000.0,
                |v| {
                    if v <= 0.0 {
                        tr("unbegrenzt", "unlimited").to_string()
                    } else {
                        format!("{v:.0}")
                    }
                },
            );
            theme::hint(ui, tr(
                "Mit Glättung filtert zusätzlich ein Median über drei Bilder einzelne Ausreißer heraus.",
                "With smoothing on, a median over three frames also removes single-frame spikes.",
            ));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                theme::field_label(ui, tr("Bildrate", "Frame rate"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let opts = [24u32, 25, 30, 40, 50, 60, 72, 100, 120, 144];
                    theme::combo(
                        ui,
                        "globalfps",
                        96.0,
                        &mut st.target_fps,
                        |v| format!("{v} fps"),
                        &opts,
                    );
                });
            });
        });
        ui.add_space(8.0);
        if ui
            .button(tr("Auf Standard zurücksetzen", "Reset to defaults"))
            .hand_cursor()
            .clicked()
        {
            let d = crate::config::Settings::default();
            st.brightness = d.brightness;
            st.saturation = d.saturation;
            st.gamma = d.gamma;
            st.smoothing_ms = d.smoothing_ms;
            st.spatial = d.spatial;
            st.slew = d.slew;
            st.target_fps = d.target_fps;
        }
        if self.cfg.settings != before {
            self.changed();
        }
    }

    fn app_tab(&mut self, ui: &mut egui::Ui) {
        let before = self.cfg.settings.clone();
        let (sw, sh) = self.screen_size();
        if capture::HAS_MONITOR_CHOICE && self.monitors.is_none() {
            self.monitors = Some(capture::monitors());
        }
        let monitors = self.monitors.clone().unwrap_or_default();
        let st = &mut self.cfg.settings;

        theme::title(ui, tr("Aufnahme", "Capture"));
        ui.add_space(4.0);
        theme::card(ui, |ui| {
            if capture::HAS_MONITOR_CHOICE && !monitors.is_empty() {
                ui.horizontal(|ui| {
                    theme::field_label(ui, tr("Monitor", "Display"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let opts: Vec<usize> = (0..monitors.len()).collect();
                        let mut m = st.monitor.min(monitors.len() - 1);
                        theme::combo(ui, "monitor", 170.0, &mut m, |i| monitors[i].clone(), &opts);
                        st.monitor = m;
                    });
                });
            }
            ui.horizontal(|ui| {
                theme::field_label(ui, tr("Auflösung", "Resolution"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let opts = [320u32, 480, 640, 960, 1280, 1600, 1920, 2560, 3840];
                    theme::combo(
                        ui,
                        "capturew",
                        150.0,
                        &mut st.capture_width,
                        |w| {
                            let h = (w as f32 * sh as f32 / sw as f32) as u32;
                            if (w as f32 / sw as f32 - 1.0).abs() < 0.02 {
                                format!("{w} × {h} ({})", tr("nativ", "native"))
                            } else {
                                format!("{w} × {h}")
                            }
                        },
                        &opts,
                    );
                });
            });
            theme::hint(
                ui,
                tr(
                    "Höher = feinere Zonen, kostet etwas mehr CPU. 320 reicht meistens.",
                    "Higher = finer zones at a little more CPU. 320 is usually enough.",
                ),
            );
            if !capture::EXCLUDES_OWN_WINDOW {
                ui.add_space(4.0);
                theme::switch(
                    ui,
                    &mut st.pause_when_focused,
                    tr(
                        "Pausieren, solange dieses Fenster aktiv ist",
                        "Pause while this window is focused",
                    ),
                )
                .on_hover_text(tr(
                    "Verhindert, dass sich das Fenster selbst aufnimmt",
                    "Prevents the window from capturing itself",
                ));
            }
            if cfg!(target_os = "linux") {
                ui.add_space(4.0);
                if ui
                    .button(tr("Anderen Monitor wählen", "Choose another display"))
                    .hand_cursor()
                    .clicked()
                {
                    st.restore_token = None;
                }
            }
        });

        ui.add_space(12.0);
        theme::title(ui, tr("Bedienung", "Editing"));
        ui.add_space(4.0);
        theme::card(ui, |ui| {
            theme::switch(
                ui,
                &mut st.snap,
                tr("Beim Ziehen einrasten", "Snap while dragging"),
            );
            theme::hint(ui, tr(
                "Pfeiltasten verschieben die Zone (mit Shift in großen Schritten), Entf löscht sie.",
                "Arrow keys move the zone (Shift for big steps), Delete removes it.",
            ));
        });

        ui.add_space(12.0);
        theme::title(ui, tr("Sprache", "Language"));
        ui.add_space(4.0);
        let mut lang = match st.language.as_str() {
            "de" => 1,
            "en" => 2,
            _ => 0,
        };
        let w = ui.available_width();
        if theme::segmented(
            ui,
            &mut lang,
            &[(0, tr("System", "System")), (1, "Deutsch"), (2, "English")],
            Some(w),
        ) {
            st.language = ["auto", "de", "en"][lang].into();
            i18n::init(&st.language);
            self.monitors = None;
        }

        ui.add_space(16.0);
        theme::title(ui, tr("Über", "About"));
        ui.add_space(4.0);
        theme::card(ui, |ui| {
            ui.label(
                RichText::new(format!("WLED Ambilight {}", crate::VERSION))
                    .font(theme::semibold(13.0)),
            );
            ui.label(
                RichText::new("© 2026 Patrick Weppelmann · GPL-3.0")
                    .size(12.0)
                    .color(theme::INK_DIM),
            );
            ui.hyperlink_to(
                RichText::new("github.com/Smotherer007/wledambilight")
                    .size(12.0)
                    .color(theme::AMBER),
                "https://github.com/Smotherer007/wledambilight",
            );
            ui.add_space(4.0);
            theme::hint(
                ui,
                tr(
                    "Einstellungen werden automatisch gespeichert in:",
                    "Settings are saved automatically to:",
                ),
            );
            ui.label(
                RichText::new(crate::config::config_path().display().to_string())
                    .size(11.0)
                    .color(theme::INK_FAINT)
                    .monospace(),
            );
        });
        if self.cfg.settings != before {
            if self.cfg.settings.restore_token.is_none() && before.restore_token.is_some() {
                // Linux: forget the token and ask the portal again
                self.shared.set_config(&self.cfg);
                self.shared.clear_restore_token();
                self.shared.request_restart();
            }
            self.changed();
        }
    }

    fn output_bar(&mut self, root: &mut egui::Ui) {
        let devices: Vec<(usize, String, bool)> = self
            .cfg
            .devices
            .iter()
            .enumerate()
            .filter(|(_, d)| d.enabled)
            .map(|(i, d)| (i, d.name.clone(), d.bgr))
            .collect();
        egui::Panel::bottom("output")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(18, 12))
                    .stroke(Stroke::new(1.0_f32, theme::LINE)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    theme::title(ui, tr("Ausgabe", "Output"));
                    if let Some((text, at)) = self.notice.as_ref() {
                        if at.elapsed() < Duration::from_secs(8) {
                            ui.add_space(10.0);
                            ui.label(RichText::new(text).size(12.0).color(theme::INK_DIM));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let s = &self.snap.stats;
                        theme::metric(
                            ui,
                            tr("Abtasten", "sampling"),
                            &format!("{} µs", s.sample_us),
                        );
                        ui.add_space(10.0);
                        theme::metric(ui, tr("raus", "out"), &format!("{:.0} fps", s.fps_out));
                        ui.add_space(10.0);
                        theme::metric(ui, tr("rein", "in"), &format!("{:.0} fps", s.fps_in));
                        ui.add_space(10.0);
                        let (w, h) = s.dims;
                        theme::metric(ui, tr("Aufnahme", "capture"), &format!("{w}×{h}"));
                    });
                });
                ui.add_space(4.0);
                if devices.is_empty() {
                    theme::hint(ui, tr("Noch kein Gerät aktiv.", "No device active yet."));
                }
                for (i, name, bgr) in devices {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(130.0, 20.0), Sense::hover());
                        ui.painter().text(
                            Pos2::new(r.left(), r.center().y),
                            egui::Align2::LEFT_CENTER,
                            &name,
                            theme::regular(12.0),
                            theme::INK_DIM,
                        );
                        let mut colors = self.snap.last.get(i).cloned().unwrap_or_default();
                        if bgr {
                            colors.iter_mut().for_each(|c| c.swap(0, 2));
                        }
                        theme::led_tape(ui, &colors, 18.0);
                    });
                }
            });
    }

    fn canvas(&mut self, root: &mut egui::Ui, live: bool) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::WALL)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(root, |ui| {
                if live || self.texture.is_none() {
                    self.update_texture(ui.ctx());
                }
                let (w, h) = self.capture_dims();
                let avail = ui.available_rect_before_wrap();
                // leave room around the picture for the light spill and the rulers
                let view = View::new(avail.shrink(28.0), w as f32 / h as f32);
                let painter = ui.painter_at(avail);

                // light on the wall
                let ranges = self.cfg.zone_ranges();
                let depth = (view.rect.width() * 0.09).clamp(30.0, 110.0);
                for (i, z) in self.cfg.zones.iter().enumerate() {
                    if !z.enabled {
                        continue;
                    }
                    let colors = self.zone_colors(i, &ranges);
                    if colors.is_empty() {
                        continue;
                    }
                    let (x0, x1) = (z.x.max(0.0), (z.x + z.w).min(1.0));
                    let (y0, y1) = (z.y.max(0.0), (z.y + z.h).min(1.0));
                    if !z.vertical {
                        if z.y <= EDGE {
                            theme::light_spill(
                                &painter,
                                [view.pos(x0, 0.0), view.pos(x1, 0.0)],
                                theme::Spill::Up,
                                &colors,
                                depth,
                            );
                        }
                        if z.y + z.h >= 1.0 - EDGE {
                            theme::light_spill(
                                &painter,
                                [view.pos(x0, 1.0), view.pos(x1, 1.0)],
                                theme::Spill::Down,
                                &colors,
                                depth,
                            );
                        }
                    } else {
                        if z.x <= EDGE {
                            theme::light_spill(
                                &painter,
                                [view.pos(0.0, y0), view.pos(0.0, y1)],
                                theme::Spill::Left,
                                &colors,
                                depth,
                            );
                        }
                        if z.x + z.w >= 1.0 - EDGE {
                            theme::light_spill(
                                &painter,
                                [view.pos(1.0, y0), view.pos(1.0, y1)],
                                theme::Spill::Right,
                                &colors,
                                depth,
                            );
                        }
                    }
                }

                // the screen
                painter.rect_filled(
                    view.rect.expand(4.0),
                    CornerRadius::same(8),
                    Color32::from_rgb(30, 33, 42),
                );
                painter.rect_filled(view.rect, CornerRadius::same(4), theme::MATTE);
                if let Some(t) = self.texture.as_ref() {
                    painter.image(
                        t.id(),
                        view.rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }

                let resp = ui.allocate_rect(view.rect, Sense::click_and_drag());
                self.draw_zones(ui, &view, &resp);
                self.canvas_overlays(ui, &view, live);
            });
    }

    fn canvas_overlays(&mut self, ui: &mut egui::Ui, view: &View, live: bool) {
        let status = self.snap.stats.status.clone();
        let painter = ui.painter_at(view.rect);
        match &status {
            CaptureStatus::Error(e) => {
                let card = Rect::from_center_size(
                    view.rect.center(),
                    Vec2::new(view.rect.width().min(440.0), 150.0),
                );
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card));
                theme::card_with(&mut child, theme::PANEL, theme::LINE_HI, |ui| {
                    ui.label(
                        RichText::new(tr("Aufnahme gestoppt", "Capture stopped"))
                            .font(theme::semibold(14.0))
                            .color(theme::CORAL),
                    );
                    ui.label(RichText::new(e).size(12.0).color(theme::INK_DIM));
                    ui.add_space(4.0);
                    if icons::labelled(
                        ui,
                        Icon::Refresh,
                        tr("Erneut versuchen", "Try again"),
                        Kind::Primary,
                    ) {
                        self.shared.request_restart();
                    }
                });
            }
            CaptureStatus::Starting | CaptureStatus::Idle => {
                theme::chip(
                    &painter,
                    Pos2::new(view.rect.left() + 10.0, view.rect.bottom() - 32.0),
                    &status_text(&status).0,
                    theme::AMBER,
                );
            }
            CaptureStatus::Running { .. } => {}
        }
        if self.cfg.zones.is_empty() && !matches!(status, CaptureStatus::Error(_)) {
            let text = if self.cfg.devices.is_empty() {
                tr(
                    "Füge links ein WLED-Gerät hinzu – es bekommt automatisch eine Zone.",
                    "Add a WLED device on the left – it gets a zone automatically.",
                )
            } else {
                tr(
                    "Lege rechts eine Zone an und ziehe sie auf den Bildbereich.",
                    "Create a zone on the right and drag it onto the picture.",
                )
            };
            let galley = painter.layout_no_wrap(text.to_owned(), theme::regular(13.0), theme::INK);
            let r =
                Rect::from_center_size(view.rect.center(), galley.size() + Vec2::new(28.0, 18.0));
            painter.rect_filled(
                r,
                CornerRadius::same(theme::R_MD),
                Color32::from_black_alpha(200),
            );
            painter.galley(r.min + Vec2::new(14.0, 9.0), galley, theme::INK);
        }
        if !live {
            theme::chip(
                &painter,
                Pos2::new(view.rect.right() - 230.0, view.rect.bottom() - 32.0),
                tr(
                    "Vorschau ruht im Hintergrund",
                    "Preview rests in background",
                ),
                theme::INK_DIM,
            );
        }
    }

    fn draw_zones(&mut self, ui: &mut egui::Ui, view: &View, resp: &egui::Response) {
        let painter = ui.painter_at(view.rect.expand(40.0));
        let ranges = self.cfg.zone_ranges();
        for (i, z) in self.cfg.zones.iter().enumerate() {
            let r = view.zone_rect(z);
            let sel = self.selected == Some(i);
            let base = Color32::from_rgb(z.color[0], z.color[1], z.color[2]);
            if sel {
                painter.rect_filled(r, CornerRadius::same(3), base.gamma_multiply(0.08));
                painter.rect_stroke(
                    r.expand(2.0),
                    CornerRadius::same(5),
                    Stroke::new(4.0_f32, base.gamma_multiply(0.22)),
                    egui::StrokeKind::Outside,
                );
            }
            painter.rect_stroke(
                r,
                CornerRadius::same(3),
                Stroke::new(
                    if sel { 2.0_f32 } else { 1.0_f32 },
                    if z.enabled {
                        base.gamma_multiply(if sel { 1.0 } else { 0.7 })
                    } else {
                        theme::INK_FAINT
                    },
                ),
                egui::StrokeKind::Inside,
            );

            let led_colors = self.zone_colors(i, &ranges);
            let at_top = z.y <= EDGE;
            let at_bottom = z.y + z.h >= 1.0 - EDGE;
            let at_left = z.x <= EDGE;
            let at_right = z.x + z.w >= 1.0 - EDGE;
            let thickness = 7.0;
            let opacity = if sel { 1.0 } else { 0.6 };
            if !led_colors.is_empty() && z.enabled {
                if !z.vertical && (at_top || at_bottom) {
                    let y0 = if at_top {
                        r.top() - thickness - 4.0
                    } else {
                        r.bottom() + 4.0
                    };
                    draw_ruler(
                        &painter,
                        Rect::from_min_max(
                            Pos2::new(r.left(), y0),
                            Pos2::new(r.right(), y0 + thickness),
                        ),
                        &led_colors,
                        false,
                        opacity,
                    );
                } else if z.vertical && (at_left || at_right) {
                    let x0 = if at_left {
                        r.left() - thickness - 4.0
                    } else {
                        r.right() + 4.0
                    };
                    draw_ruler(
                        &painter,
                        Rect::from_min_max(
                            Pos2::new(x0, r.top()),
                            Pos2::new(x0 + thickness, r.bottom()),
                        ),
                        &led_colors,
                        true,
                        opacity,
                    );
                } else if z.vertical {
                    draw_ruler(
                        &painter,
                        Rect::from_center_size(r.center(), Vec2::new(10.0, r.height())),
                        &led_colors,
                        true,
                        opacity,
                    );
                } else {
                    draw_ruler(
                        &painter,
                        Rect::from_center_size(r.center(), Vec2::new(r.width(), 10.0)),
                        &led_colors,
                        false,
                        opacity,
                    );
                }
            }

            // direction: chevron at LED 0 pointing towards the last LED
            if r.width() > 26.0 && r.height() > 12.0 {
                let reversed = self
                    .cfg
                    .device(&z.device)
                    .map(|d| d.reverse)
                    .unwrap_or(false);
                let s = if reversed { 4.0 } else { -4.0 };
                let chevron = if z.vertical {
                    let y = if reversed {
                        r.bottom() - 10.0
                    } else {
                        r.top() + 10.0
                    };
                    vec![
                        Pos2::new(r.center().x - 4.0, y + s),
                        Pos2::new(r.center().x, y),
                        Pos2::new(r.center().x + 4.0, y + s),
                    ]
                } else {
                    let x = if reversed {
                        r.right() - 10.0
                    } else {
                        r.left() + 10.0
                    };
                    vec![
                        Pos2::new(x + s, r.center().y - 4.0),
                        Pos2::new(x, r.center().y),
                        Pos2::new(x + s, r.center().y + 4.0),
                    ]
                };
                painter.add(egui::Shape::line(
                    chevron,
                    Stroke::new(1.5_f32, Color32::from_white_alpha(210)),
                ));
            }

            if sel {
                for c in [
                    r.left_top(),
                    r.center_top(),
                    r.right_top(),
                    r.right_center(),
                    r.right_bottom(),
                    r.center_bottom(),
                    r.left_bottom(),
                    r.left_center(),
                ] {
                    painter.circle(c, 4.0, Color32::WHITE, Stroke::new(1.5_f32, base));
                }
                let (lo, hi) = ranges.get(i).copied().unwrap_or((0, 0));
                let label = format!("{} · LED {}–{}", z.name, lo, hi.saturating_sub(1));
                let tag_y = if at_top {
                    r.bottom() + 8.0
                } else if at_bottom {
                    r.top() - 30.0
                } else {
                    r.top() + 6.0
                };
                theme::chip(
                    &painter,
                    Pos2::new(r.left() + 6.0, tag_y),
                    &label,
                    theme::INK,
                );
            } else {
                let badge = Rect::from_center_size(
                    Pos2::new(r.left() + 10.0, r.top() + 10.0),
                    Vec2::splat(16.0),
                );
                painter.rect_filled(badge, CornerRadius::same(4), Color32::from_black_alpha(170));
                painter.text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", i + 1),
                    theme::semibold(10.5),
                    base,
                );
            }
        }

        // interaction
        let pointer = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
        let pressed = ui.ctx().input(|i| i.pointer.primary_down());
        if resp.drag_started() && pressed {
            if let Some(p) = resp.interact_pointer_pos() {
                // the selected zone wins, then the topmost one
                let order: Vec<usize> = self
                    .selected
                    .into_iter()
                    .chain(
                        (0..self.cfg.zones.len())
                            .rev()
                            .filter(|i| Some(*i) != self.selected),
                    )
                    .collect();
                for i in order {
                    let Some(z) = self.cfg.zones.get(i) else {
                        continue;
                    };
                    if let Some(grab) = hit(view.zone_rect(z), p) {
                        self.selected = Some(i);
                        self.drag = Some(DragState {
                            zone: i,
                            grab,
                            start: p,
                            orig: (z.x, z.y, z.w, z.h),
                        });
                        break;
                    }
                }
            }
        }
        if resp.drag_stopped() || !pressed {
            self.drag = None;
        }
        if let (Some(d), Some(p)) = (self.drag, resp.interact_pointer_pos()) {
            self.apply_drag(view, &d, p);
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.selected = (0..self.cfg.zones.len())
                    .rev()
                    .find(|i| view.zone_rect(&self.cfg.zones[*i]).contains(p));
            }
        }
        if let Some(p) = pointer {
            let mut cursor = egui::CursorIcon::Default;
            let grab = self.drag.map(|d| d.grab).or_else(|| {
                self.cfg
                    .zones
                    .iter()
                    .rev()
                    .find_map(|z| hit(view.zone_rect(z), p))
            });
            if let Some(g) = grab {
                cursor = match g {
                    Grab::Move => egui::CursorIcon::Move,
                    Grab::N | Grab::S => egui::CursorIcon::ResizeVertical,
                    Grab::E | Grab::W => egui::CursorIcon::ResizeHorizontal,
                    Grab::NW | Grab::SE => egui::CursorIcon::ResizeNwSe,
                    Grab::NE | Grab::SW => egui::CursorIcon::ResizeNeSw,
                };
            }
            if resp.hovered() || self.drag.is_some() {
                ui.ctx().set_cursor_icon(cursor);
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        let focused = ctx.input(|i| i.focused);
        // Linux: freeze what the window shows while it is in the background
        let live = capture::EXCLUDES_OWN_WINDOW || focused || capture::demo_mode();
        let manual = self.tray_state.manual_pause.load(Ordering::Relaxed);
        let want_pause = manual
            || (!capture::EXCLUDES_OWN_WINDOW && self.cfg.settings.pause_when_focused && focused);
        self.shared.paused.store(want_pause, Ordering::Relaxed);

        // a frozen window still needs one picture after startup
        let never_seen =
            self.texture.is_none() || matches!(self.snap.stats.status, CaptureStatus::Idle);
        if live || never_seen {
            self.snap.last.clone_from(&self.shared.last.lock().unwrap());
            self.snap.stats = self.shared.stats();
        }
        // keep the portal token in our copy of the config so saving keeps it
        if let Some(t) = self.shared.restore_token() {
            if self.cfg.settings.restore_token.as_ref() != Some(&t) {
                self.cfg.settings.restore_token = Some(t);
                self.pushed.settings.restore_token = self.cfg.settings.restore_token.clone();
                self.config_dirty = true;
            }
        }

        if std::mem::take(&mut self.start_minimized) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        // --- window lifecycle driven by the tray ---
        if self.tray_state.show.swap(false, Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if self.tray_state.quit.load(Ordering::Relaxed) && !self.quitting {
            self.quitting = true;
            self.cfg.save();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            if self.tray.is_some() {
                // closing the window keeps the ambilight running in the tray
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                self.shared.paused.store(manual, Ordering::Relaxed);
            } else {
                // no tray to come back from: really quit
                self.quitting = true;
                self.cfg.save();
            }
        }

        // tray tooltip / checkmark, only when something changed
        let tray_status = {
            let s = &self.snap.stats;
            let (t, _) = status_text(&s.status);
            format!(
                "{t} · {:.0} fps{}",
                s.fps_out,
                if want_pause {
                    tr(" · pausiert", " · paused")
                } else {
                    ""
                }
            )
        };
        if self.tray_seen != (tray_status.clone(), manual) {
            *self.tray_state.status.lock().unwrap() = tray_status.clone();
            self.tray_seen = (tray_status, manual);
            if let Some(t) = self.tray.as_ref() {
                t.refresh();
            }
        }

        self.keyboard(ctx);
        self.top_bar(root);
        self.output_bar(root);
        self.devices_panel(root);
        self.inspector(root);
        self.canvas(root, live);

        if self.pushed != self.cfg {
            self.changed();
        }
        self.save_if_due();
        ctx.request_repaint_after(Duration::from_millis(match (focused, live) {
            (true, _) => 16,
            (false, true) => 100,
            (false, false) => 250,
        }));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::WALL.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self) {
        self.cfg.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_counts_add_up() {
        for total in [4u16, 7, 60, 143, 300] {
            let c = split_counts(total, 9.0 / 16.0);
            assert_eq!(
                c.iter().map(|v| *v as u32).sum::<u32>(),
                total as u32,
                "{total}: {c:?}"
            );
            assert!(c[0] >= c[1], "long edges get more LEDs: {c:?}");
        }
    }

    #[test]
    fn hit_finds_corners_and_inside() {
        let r = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 100.0));
        assert!(matches!(hit(r, Pos2::new(101.0, 101.0)), Some(Grab::NW)));
        assert!(matches!(hit(r, Pos2::new(200.0, 150.0)), Some(Grab::Move)));
        assert!(matches!(hit(r, Pos2::new(299.0, 150.0)), Some(Grab::E)));
        assert!(hit(r, Pos2::new(10.0, 10.0)).is_none());
    }
}
