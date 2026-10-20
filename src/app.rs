use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Color32, Pos2, Rect, Sense, TextureHandle, Vec2};

use crate::capture::{CaptureStatus, SharedFrame};
use crate::config::{Config, DeviceCfg, Protocol, Zone, ZonePreset};
use crate::wled::{self, WledInfo};
use crate::icons::{self, Icon};
use crate::theme::{self, HandCursor};
use crate::tray::{self, TrayState};
use crate::worker::{Pipeline, Shared};

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

struct DragState {
    zone: usize,
    grab: Grab,
    start: Pos2,
    orig: (f32, f32, f32, f32),
}

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
        let rect = Rect::from_center_size(avail.center(), size);
        Self { rect }
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

pub struct App {
    pub cfg: Config,
    pipeline: Option<Pipeline>,
    shared: Arc<Shared>,
    texture: Option<TextureHandle>,
    texture_seq: u64,
    drag: Option<DragState>,
    selected: Option<usize>,
    scan: Option<(Arc<Mutex<Vec<WledInfo>>>, Arc<Mutex<bool>>)>,
    status: String,
    config_dirty: bool,
    last_save: Instant,
    last_hosts: Vec<String>,
    _tray: Option<ksni::blocking::Handle<crate::tray::Tray>>,
    tray_state: TrayState,
    quitting: bool,
    // snapshot shown while the window is unfocused, so the window stays pixel
    // identical and cannot feed its own output back into the capture
    show_last: Vec<Vec<[u8; 3]>>,
    show_fps_in: f32,
    show_fps_out: f32,
    show_sample: u128,
    show_status: CaptureStatus,
    show_dims: (u32, u32),
    /// what the pipeline thread currently knows - compared every frame
    pushed: Config,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let cfg = Config::load();
        let pipeline = Pipeline::start(&cfg);
        let shared = pipeline.shared.clone();
        let mut tray_state = TrayState::new(shared.paused.clone());
        tray_state.ctx = Arc::new(Mutex::new(Some(cc.egui_ctx.clone())));
        let tray = tray::spawn(tray_state.clone(), cc.egui_ctx.clone());
        let mut app = Self {
            pipeline: Some(pipeline),
            shared,
            texture: None,
            texture_seq: u64::MAX,
            drag: None,
            selected: if cfg.zones.is_empty() { None } else { Some(0) },
            scan: None,
            status: String::new(),
            config_dirty: false,
            last_save: Instant::now(),
            last_hosts: Vec::new(),
            _tray: tray,
            tray_state,
            quitting: false,
            show_last: Vec::new(),
            show_fps_in: 0.0,
            show_fps_out: 0.0,
            show_sample: 0,
            show_status: CaptureStatus::Idle,
            show_dims: (320, 180),
            pushed: cfg.clone(),
            cfg,
        };
        app.sync_config();
        app
    }

    /// Push the edited config into the pipeline thread.
    fn sync_config(&mut self) {
        if let Some(p) = self.pipeline.as_ref() {
            p.shared.set_config(&self.cfg);
        }
    }

    fn capture_dims(&self) -> (u32, u32) {
        let d = *self.shared.dims.lock().unwrap();
        if d.0 > 8 && d.1 > 8 {
            d
        } else {
            let w = self.cfg.settings.capture_width.clamp(64, 1280);
            (w, ((w as f32 * 9.0 / 16.0) as u32).max(32))
        }
    }

    fn capture_status(&self) -> CaptureStatus {
        self.shared.status.lock().unwrap().clone()
    }

    fn update_texture(&mut self, ui: &mut egui::Ui, frame: Option<&SharedFrame>) {
        let Some(f) = frame else { return };
        if f.seq == self.texture_seq && self.texture.is_some() {
            return;
        }
        self.texture_seq = f.seq;
        let img = egui::ColorImage::from_rgb([f.w as usize, f.h as usize], &f.data);
        match self.texture.as_mut() {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => {
                self.texture = Some(ui.ctx().load_texture(
                    "preview",
                    img,
                    egui::TextureOptions::LINEAR,
                ))
            }
        }
    }

    fn save_if_due(&mut self) {
        if self.config_dirty && self.last_save.elapsed() > Duration::from_millis(300) {
            self.cfg.save();
            self.sync_config();
            self.config_dirty = false;
            self.last_save = Instant::now();
        }
    }

    fn next_zone_color(&self) -> [u8; 3] {
        const PALETTE: [[u8; 3]; 6] = [
            [255, 80, 80],
            [80, 255, 130],
            [90, 150, 255],
            [255, 210, 80],
            [220, 120, 255],
            [80, 230, 230],
        ];
        PALETTE[self.cfg.zones.len() % PALETTE.len()]
    }

    /// A new device starts as one zone that uses the whole strip.
    fn add_device(&mut self, host: String, name: String, leds: u16) {
        if self.cfg.device(&host).is_some() {
            self.status = format!("{host} is already in the list");
            return;
        }
        let dev = DeviceCfg::new(&host, name.clone(), leds);
        let id = dev.id.clone();
        let edge = self.next_free_edge();
        let mut zone = Zone::preset(edge, id, leds.max(1), self.next_zone_color());
        zone.name = name.clone();
        self.cfg.devices.push(dev);
        self.cfg.zones.push(zone);
        self.selected = Some(self.cfg.zones.len() - 1);
        self.status = format!("{} added - one zone with all {} LEDs", name, leds);
        self.sync_config();
        self.config_dirty = true;
    }

    /// Add another zone on the selected device, using the LEDs that are still free.
    fn add_free_zone(&mut self) {
        let device = self
            .selected
            .and_then(|i| self.cfg.zones.get(i))
            .map(|z| z.device.clone())
            .or_else(|| self.cfg.devices.iter().find(|d| d.enabled).map(|d| d.id.clone()));
        let Some(device) = device else {
            self.status = "add a WLED device first".into();
            return;
        };
        let Some(dev) = self.cfg.device(&device).cloned() else {
            return;
        };
        let free = dev.led_count.saturating_sub(self.cfg.used_leds(&device, None));
        let count = if free > 0 { free } else { (dev.led_count / 4).max(1) };
        let mut zone = Zone::preset(ZonePreset::Top, device, count, self.next_zone_color());
        zone.name = format!("Zone {}", self.cfg.zones.len() + 1);
        zone.y = 0.35;
        zone.h = 0.3;
        self.cfg.zones.push(zone);
        self.selected = Some(self.cfg.zones.len() - 1);
        self.sync_config();
        self.config_dirty = true;
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
            let (x, y, w, _h) = match kind {
                ZonePreset::Top => (0.0, 0.0, 1.0, 0.12f32),
                ZonePreset::Bottom => (0.0, 0.88, 1.0, 0.12),
                ZonePreset::Left => (0.0, 0.0, 0.12, 1.0),
                _ => (0.88, 0.0, 0.12, 1.0),
            };
            let taken = self.cfg.zones.iter().any(|z| {
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
            self.status = "no active device".into();
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
        let palette = [
            [255, 80, 80],
            [80, 255, 130],
            [90, 150, 255],
            [255, 210, 80],
        ];
        let ids: Vec<String> = devices.iter().map(|d| d.id.clone()).collect();
        self.cfg.zones.retain(|z| !ids.contains(&z.device));
        for (i, dev) in devices.iter().enumerate() {
            let kind = edges[i % edges.len()];
            let mut z = Zone::preset(
                kind,
                dev.id.clone(),
                dev.led_count.max(1),
                palette[i % palette.len()],
            );
            z.name = dev.name.clone();
            self.cfg.zones.push(z);
        }
        self.selected = Some(0);
        self.status = format!(
            "{} device(s) placed on the screen edges (full strip length each)",
            devices.len()
        );
        self.sync_config();
        self.config_dirty = true;
    }

    /// The "split" feature: turn the selected zone's device into four edge zones,
    /// LED count shared out by edge length.
    fn split_into_edges(&mut self) {
        let Some(idx) = self.selected else {
            self.status = "select the zone you want to split".into();
            return;
        };
        let Some(zone) = self.cfg.zones.get(idx).cloned() else {
            return;
        };
        let Some(dev) = self.cfg.device(&zone.device).cloned() else {
            return;
        };
        let (cw, ch) = self.capture_dims();
        let ratio = ch as f32 / cw as f32;
        let lens = [1.0f32, ratio, 1.0, ratio];
        let sum: f32 = lens.iter().sum();
        let total = dev.led_count as f32;
        let mut counts: Vec<u16> = lens
            .iter()
            .map(|l| (l / sum * total).floor().max(1.0) as u16)
            .collect();
        let mut diff = dev.led_count as i32 - counts.iter().map(|c| *c as i32).sum::<i32>();
        while diff > 0 {
            let mut best = 0usize;
            for i in 1..4 {
                if lens[i] / sum * total - counts[i] as f32
                    > lens[best] / sum * total - counts[best] as f32
                {
                    best = i;
                }
            }
            counts[best] += 1;
            diff -= 1;
        }
        while diff < 0 {
            if let Some(i) = (0..4).max_by_key(|i| counts[*i]) {
                if counts[i] > 1 {
                    counts[i] -= 1;
                    diff += 1;
                    continue;
                }
            }
            break;
        }
        // zones of this device are replaced, otherwise the LED ranges would clash
        let first = self
            .cfg
            .zones
            .iter()
            .position(|z| z.device == dev.id)
            .unwrap_or(idx);
        self.cfg.zones.retain(|z| z.device != dev.id);
        let palette = [
            [255, 80, 80],
            [80, 255, 130],
            [90, 150, 255],
            [255, 210, 80],
        ];
        let kinds = [ZonePreset::Top, ZonePreset::Right, ZonePreset::Bottom, ZonePreset::Left];
        for (i, kind) in kinds.iter().enumerate() {
            let z = Zone::preset(*kind, dev.id.clone(), counts[i], palette[i]);
            self.cfg.zones.insert(first + i, z);
        }
        self.selected = Some(first);
        self.status = format!(
            "{} split into 4 edges: {} / {} / {} / {} LEDs",
            dev.name, counts[0], counts[1], counts[2], counts[3]
        );
        self.sync_config();
        self.config_dirty = true;
    }


    fn apply_drag(&mut self, view: &View, drag: &DragState, pos: Pos2) {
        let (mut nx, mut ny) = view.norm(pos);
        nx = nx.clamp(-0.5, 1.5);
        ny = ny.clamp(-0.5, 1.5);
        let (sx, sy) = (drag.start.x, drag.start.y);
        let _ = (sx, sy);
        let (dx, dy) = {
            let (ox, oy) = view.norm(drag.start);
            (nx - ox, ny - oy)
        };
        let (ox, oy, ow, oh) = drag.orig;
        let (mut x, mut y, mut w, mut h) = (ox, oy, ow, oh);
        match drag.grab {
            Grab::Move => {
                x = ox + dx;
                y = oy + dy;
            }
            Grab::W => {
                x = ox + dx;
                w = ow - dx;
            }
            Grab::E => w = ow + dx,
            Grab::N => {
                y = oy + dy;
                h = oh - dy;
            }
            Grab::S => h = oh + dy,
            Grab::NW => {
                x = ox + dx;
                w = ow - dx;
                y = oy + dy;
                h = oh - dy;
            }
            Grab::NE => {
                w = ow + dx;
                y = oy + dy;
                h = oh - dy;
            }
            Grab::SW => {
                x = ox + dx;
                w = ow - dx;
                h = oh + dy;
            }
            Grab::SE => {
                w = ow + dx;
                h = oh + dy;
            }
        }
        // one capture pixel is the smallest unit - at 320x180 that is ~8 screen
        // pixels, at 1280x720 it is ~2. Raise the capture width for finer control.
        let (cap_w, cap_h) = self.capture_dims();
        let min_w = 1.0 / cap_w.max(1) as f32;
        let min_h = 1.0 / cap_h.max(1) as f32;
        if w < min_w {
            if drag.grab == Grab::W || drag.grab == Grab::NW || drag.grab == Grab::SW {
                x = ox + ow - min_w;
            }
            w = min_w;
        }
        if h < min_h {
            if drag.grab == Grab::N || drag.grab == Grab::NW || drag.grab == Grab::NE {
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
                if Some(i) != self.selected {
                    xs.push(z.x);
                    xs.push(z.x + z.w);
                    ys.push(z.y);
                    ys.push(z.y + z.h);
                }
            }
            match drag.grab {
                Grab::Move => {
                    if let Some(v) = xs.iter().find(|v| (x - **v).abs() < t) {
                        x = *v;
                    } else if let Some(v) = xs.iter().find(|v| (x + w - **v).abs() < t) {
                        x = *v - w;
                    }
                    if let Some(v) = ys.iter().find(|v| (y - **v).abs() < ty) {
                        y = *v;
                    } else if let Some(v) = ys.iter().find(|v| (y + h - **v).abs() < ty) {
                        y = *v - h;
                    }
                }
                _ => {
                    if matches!(drag.grab, Grab::W | Grab::NW | Grab::SW) {
                        if let Some(v) = xs.iter().find(|v| (x - **v).abs() < t) {
                            let nw = x + w - *v;
                            x = *v;
                            w = nw;
                        }
                    }
                    if matches!(drag.grab, Grab::E | Grab::NE | Grab::SE) {
                        if let Some(v) = xs.iter().find(|v| (x + w - **v).abs() < t) {
                            w = *v - x;
                        }
                    }
                    if matches!(drag.grab, Grab::N | Grab::NW | Grab::NE) {
                        if let Some(v) = ys.iter().find(|v| (y - **v).abs() < ty) {
                            let nh = y + h - *v;
                            y = *v;
                            h = nh;
                        }
                    }
                    if matches!(drag.grab, Grab::S | Grab::SW | Grab::SE) {
                        if let Some(v) = ys.iter().find(|v| (y + h - **v).abs() < ty) {
                            h = *v - y;
                        }
                    }
                }
            }
        }
        let maxx = 1.0;
        let maxy = 1.0;
        x = x.clamp(-w + min_w, maxx - min_w);
        y = y.clamp(-h + min_h, maxy - min_h);
        w = w.clamp(min_w, maxx - x.max(0.0));
        h = h.clamp(min_h, maxy - y.max(0.0));
        if let Some(z) = self.cfg.zones.get_mut(drag.zone) {
            z.x = x;
            z.y = y;
            z.w = w;
            z.h = h;
        }
        self.sync_config();
        self.config_dirty = true;
    }

    fn draw_zones(&mut self, ui: &mut egui::Ui, view: &View, resp: &egui::Response) -> bool {
        let painter = ui.painter_at(view.rect.expand(40.0));
        // ---- zone overlays -------------------------------------------------
        // No big translucent blocks over the picture. The zones are outlines;
        // the interesting part is the LED ruler: one cell per LED, filled with
        // the colour that is actually being sent.
        let ranges = self.cfg.zone_ranges();
        for (i, z) in self.cfg.zones.iter().enumerate() {
            let r = view.zone_rect(z);
            let sel = self.selected == Some(i);
            let base = Color32::from_rgb(z.color[0], z.color[1], z.color[2]);

            if sel {
                painter.rect_filled(
                    r,
                    egui::CornerRadius::same(4),
                    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 16),
                );
            }
            if sel {
                painter.rect_stroke(
                    r.expand(2.0),
                    egui::CornerRadius::same(6),
                    egui::Stroke::new(4.0_f32, base.gamma_multiply(0.25)),
                    egui::StrokeKind::Outside,
                );
            }
            painter.rect_stroke(
                r,
                egui::CornerRadius::same(4),
                egui::Stroke::new(
                    if sel { 2.0_f32 } else { 1.0_f32 },
                    if z.enabled {
                        base.gamma_multiply(if sel { 1.2 } else { 0.7 })
                    } else {
                        theme::TEXT_FAINT
                    },
                ),
                egui::StrokeKind::Inside,
            );

            // LED colours of this zone
            let dev_idx = self
                .cfg
                .devices
                .iter()
                .position(|d| d.id == z.device);
            let (lo, hi) = ranges.get(i).copied().unwrap_or((0, 0));
            let mut led_colors: Vec<Color32> = Vec::new();
            if let Some(di) = dev_idx {
                if let Some(all) = self.show_last.get(di) {
                    for k in lo..hi {
                        if let Some(c) = all.get(k as usize) {
                            led_colors.push(Color32::from_rgb(c[0], c[1], c[2]));
                        }
                    }
                }
            }

            // where does this zone sit? -> the ruler goes to that picture edge
            const EDGE: f32 = 0.02;
            let at_top = z.y <= EDGE;
            let at_bottom = z.y + z.h >= 1.0 - EDGE;
            let at_left = z.x <= EDGE;
            let at_right = z.x + z.w >= 1.0 - EDGE;
            let thickness = 7.0;

            if !led_colors.is_empty() && (sel || z.enabled) {
                if !z.vertical && (at_top || at_bottom) {
                    let y0 = if at_top {
                        r.top() - thickness - 3.0
                    } else {
                        r.bottom() + 3.0
                    };
                    draw_ruler(
                        &painter,
                        egui::Rect::from_min_max(
                            Pos2::new(r.left(), y0),
                            Pos2::new(r.right(), y0 + thickness),
                        ),
                        &led_colors,
                        false,
                        if sel { 1.0 } else { 0.55 },
                    );
                } else if z.vertical && (at_left || at_right) {
                    let x0 = if at_left {
                        r.left() - thickness - 3.0
                    } else {
                        r.right() + 3.0
                    };
                    draw_ruler(
                        &painter,
                        egui::Rect::from_min_max(
                            Pos2::new(x0, r.top()),
                            Pos2::new(x0 + thickness, r.bottom()),
                        ),
                        &led_colors,
                        true,
                        if sel { 1.0 } else { 0.55 },
                    );
                } else {
                    // free floating zone: ruler along its centre line
                    if z.vertical {
                        draw_ruler(
                            &painter,
                            egui::Rect::from_center_size(r.center(), Vec2::new(10.0, r.height())),
                            &led_colors,
                            true,
                            if sel { 1.0 } else { 0.55 },
                        );
                    } else {
                        draw_ruler(
                            &painter,
                            egui::Rect::from_center_size(r.center(), Vec2::new(r.width(), 10.0)),
                            &led_colors,
                            false,
                            if sel { 1.0 } else { 0.55 },
                        );
                    }
                }
            }

            // direction: chevrons pointing from LED 0 towards the last LED
            if r.width() > 26.0 && r.height() > 12.0 {
                let reversed = self
                    .cfg
                    .device(&z.device)
                    .map(|d| d.reverse)
                    .unwrap_or(false);
                let chevron = if z.vertical {
                    let y = if reversed { r.bottom() - 10.0 } else { r.top() + 10.0 };
                    vec![
                        Pos2::new(r.center().x - 4.0, y + if reversed { 4.0 } else { -4.0 }),
                        Pos2::new(r.center().x, y),
                        Pos2::new(r.center().x + 4.0, y + if reversed { 4.0 } else { -4.0 }),
                    ]
                } else {
                    let x = if reversed { r.right() - 10.0 } else { r.left() + 10.0 };
                    vec![
                        Pos2::new(x + if reversed { 4.0 } else { -4.0 }, r.center().y - 4.0),
                        Pos2::new(x, r.center().y),
                        Pos2::new(x + if reversed { 4.0 } else { -4.0 }, r.center().y + 4.0),
                    ]
                };
                painter.add(egui::Shape::line(
                    chevron,
                    egui::Stroke::new(1.5_f32, Color32::from_white_alpha(200)),
                ));
            }

            // handles only for the selected zone
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
                    let h = egui::Rect::from_center_size(c, Vec2::splat(7.0));
                    painter.rect_filled(h, egui::CornerRadius::same(2), Color32::WHITE);
                    painter.rect_stroke(
                        h,
                        egui::CornerRadius::same(2),
                        egui::Stroke::new(1.0_f32, Color32::from_black_alpha(140)),
                        egui::StrokeKind::Outside,
                    );
                }
            }

            // compact tag: selected zone always, others only as a small badge
            if sel {
                let label = format!(
                    "{} · {}–{} · {}",
                    z.name,
                    lo,
                    hi.saturating_sub(1),
                    self.zone_device_name(z)
                );
                let tag_y = if at_top {
                    r.bottom() + 8.0
                } else if at_bottom {
                    r.top() - 22.0
                } else {
                    r.top() + 6.0
                };
                theme::chip(
                    &painter,
                    Pos2::new(r.left() + 6.0, tag_y),
                    &label,
                    Color32::from_rgb(235, 238, 246),
                );
            } else {
                let badge = egui::Rect::from_center_size(
                    Pos2::new(r.left() + 8.0, r.top() + 8.0),
                    Vec2::splat(14.0),
                );
                painter.rect_filled(
                    badge,
                    egui::CornerRadius::same(4),
                    Color32::from_black_alpha(150),
                );
                painter.text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", i + 1),
                    egui::FontId::proportional(10.0),
                    base,
                );
            }
        }

        // interaction
        let pointer = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
        let pressed = ui.ctx().input(|i| i.pointer.primary_down());
        if resp.drag_started() && pressed {
            if let Some(p) = resp.interact_pointer_pos() {
                let order: Vec<usize> = std::iter::once(self.selected)
                    .flatten()
                    .chain((0..self.cfg.zones.len()).rev().filter(|i| Some(*i) != self.selected))
                    .collect();
                for i in order {
                    if i >= self.cfg.zones.len() {
                        continue;
                    }
                    let r = view.zone_rect(&self.cfg.zones[i]);
                    if let Some(grab) = hit(r, p) {
                        self.selected = Some(i);
                        let z = &self.cfg.zones[i];
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
        if resp.drag_stopped() {
            self.drag = None;
        }
        if let (Some(d), Some(p)) = (self.drag.as_ref(), resp.interact_pointer_pos()) {
            // guard against implausible jumps (layout changes, stale pointers)
            let jump = ((p.x - d.start.x).powi(2) + (p.y - d.start.y).powi(2)).sqrt();
            if jump > view.rect.width().max(1.0) * 0.6 || !pressed {
                self.drag = None;
            }
        }
        if let (Some(d), Some(p)) = (self.drag.as_ref(), resp.interact_pointer_pos()) {
            let (zone, grab, start, orig) = (d.zone, d.grab, d.start, d.orig);
            let st = DragState {
                zone,
                grab,
                start,
                orig,
            };
            self.apply_drag(view, &st, p);
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let mut hit_zone = None;
                for i in (0..self.cfg.zones.len()).rev() {
                    if view.zone_rect(&self.cfg.zones[i]).contains(p) {
                        hit_zone = Some(i);
                        break;
                    }
                }
                self.selected = hit_zone;
            }
        }
        // cursor shape
        if let Some(p) = pointer {
            let mut cursor = egui::CursorIcon::Default;
            for (i, z) in self.cfg.zones.iter().enumerate().rev() {
                if Some(i) == self.selected || self.drag.is_some() {
                    // only show handles for the selected zone
                }
                let r = view.zone_rect(z);
                if let Some(g) = hit(r, p) {
                    cursor = match g {
                        Grab::Move => egui::CursorIcon::Move,
                        Grab::N | Grab::S => egui::CursorIcon::ResizeVertical,
                        Grab::E | Grab::W => egui::CursorIcon::ResizeHorizontal,
                        Grab::NW | Grab::SE => egui::CursorIcon::ResizeNwSe,
                        Grab::NE | Grab::SW => egui::CursorIcon::ResizeNeSw,
                    };
                    break;
                }
            }
            ui.ctx().set_cursor_icon(cursor);
        }
        false
    }

    fn zone_device_name(&self, z: &Zone) -> String {
        self.cfg
            .device(&z.device)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "no device".into())
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        let Some(i) = self.selected else { return };
        if i >= self.cfg.zones.len() {
            return;
        }
        let (mut dx, mut dy) = (0.0f32, 0.0f32);
        let step = ctx.input(|inp| if inp.modifiers.shift { 0.02 } else { 0.002 });
        ctx.input(|inp| {
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
        });
        if dx != 0.0 || dy != 0.0 {
            if let Some(z) = self.cfg.zones.get_mut(i) {
                z.x = (z.x + dx).clamp(-z.w + 0.01, 1.0 - 0.01);
                z.y = (z.y + dy).clamp(-z.h + 0.01, 1.0 - 0.01);
            }
            self.sync_config();
            self.config_dirty = true;
        }
    }
}

/// One cell per LED, filled with the colour that is currently being sent.
fn draw_ruler(
    painter: &egui::Painter,
    rect: Rect,
    colors: &[Color32],
    vertical: bool,
    opacity: f32,
) {
    painter.rect_filled(
        rect.expand(1.0),
        egui::CornerRadius::same(3),
        Color32::from_black_alpha(190),
    );
    let n = colors.len() as f32;
    for (i, c) in colors.iter().enumerate() {
        let a = i as f32 / n;
        let b = (i + 1) as f32 / n;
        if vertical {
            let y0 = rect.top() + a * rect.height();
            let y1 = rect.top() + b * rect.height();
            painter.rect_filled(
                egui::Rect::from_min_max(
                    Pos2::new(rect.left(), y0),
                    Pos2::new(rect.right(), (y1 - 0.6).max(y0 + 0.4)),
                ),
                0,
                c.gamma_multiply(opacity),
            );
        } else {
            let x0 = rect.left() + a * rect.width();
            let x1 = rect.left() + b * rect.width();
            painter.rect_filled(
                egui::Rect::from_min_max(
                    Pos2::new(x0, rect.top()),
                    Pos2::new((x1 - 0.6).max(x0 + 0.4), rect.bottom()),
                ),
                0,
                c.gamma_multiply(opacity),
            );
        }
    }
    painter.rect_stroke(
        rect.expand(1.0),
        egui::CornerRadius::same(3),
        egui::Stroke::new(1.0_f32, theme::BORDER_HI),
        egui::StrokeKind::Outside,
    );
}

fn hit(r: Rect, p: Pos2) -> Option<Grab> {
    if !r.expand(4.0).contains(p) {
        return None;
    }
    let m = 9.0;
    let near = |a: f32, b: f32| (a - b).abs() < m;
    let l = near(p.x, r.left());
    let rr = near(p.x, r.right());
    let t = near(p.y, r.top());
    let b = near(p.y, r.bottom());
    if r.width() < 8.0 || r.height() < 8.0 {
        if r.expand(6.0).contains(p) {
            return Some(Grab::Move);
        }
    }
    match (l, rr, t, b) {
        (true, _, true, _) => Some(Grab::NW),
        (_, true, true, _) => Some(Grab::NE),
        (true, _, _, true) => Some(Grab::SW),
        (_, true, _, true) => Some(Grab::SE),
        (true, _, _, _) => Some(Grab::W),
        (_, true, _, _) => Some(Grab::E),
        (_, _, true, _) => Some(Grab::N),
        (_, _, _, true) => Some(Grab::S),
        _ if r.expand(6.0).contains(p) => Some(Grab::Move),
        _ => None,
    }
}

// ---------------------------------------------------------------- user interface

impl App {
    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("chrome")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Ambilight")
                            .size(16.5)
                            .strong()
                            .color(theme::TEXT),
                    );
                    theme::pill(ui, "WLED", theme::ACCENT, true);
                    ui.add_space(14.0);

                    ui.add_space(12.0);

                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if icons::labelled(ui, Icon::Save, "Speichern", true) {
                                self.cfg.save();
                                self.config_dirty = false;
                                self.status = format!("gespeichert: {}", crate::config::config_path().display());
                            }
                            if icons::labelled(ui, Icon::Refresh, "Aufnahme", false) {
                                self.shared.request_restart();
                            }
                            let paused = self
                                .tray_state
                                .manual_pause
                                .load(std::sync::atomic::Ordering::Relaxed);
                            if icons::labelled(
                                ui,
                                if paused { Icon::Play } else { Icon::Pause },
                                if paused { "Weiter" } else { "Pause" },
                                false,
                            ) {
                                self.tray_state.manual_pause.store(
                                    !paused,
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                            }
                            ui.add_space(6.0);
                            let (dot_color, label) = if self.shared.paused.load(std::sync::atomic::Ordering::Relaxed) {
                                (theme::WARN, format!("{:.0} fps", self.show_fps_out))
                            } else {
                                (theme::GOOD, format!("{:.0} fps", self.show_fps_out))
                            };
                            theme::pill(ui, label, dot_color, true);
                        },
                    );
                });
            });
    }

    fn devices_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("devices")
            .resizable(true)
            .default_width(330.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            theme::section(ui, "Geräte");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if icons::button(ui, Icon::Search, "Geräte suchen") {
                                        let scan = wled::search();
                                        self.scan = Some((scan.found, scan.done));
                                        self.status = "Suche läuft…".into();
                                    }
                                },
                            );
                        });
                        let mut devices = std::mem::take(&mut self.cfg.devices);
                        let mut remove: Option<usize> = None;
                        for (i, dev) in devices.iter_mut().enumerate() {
                            let used = self.cfg.used_leds(&dev.id, None);
                            theme::card(ui, |ui| {
                                ui.horizontal(|ui| {
                                    theme::dot(
                                        ui,
                                        if dev.enabled { theme::GOOD } else { theme::TEXT_FAINT },
                                    );
                                    ui.label(
                                        egui::RichText::new(&dev.name)
                                            .size(14.0)
                                            .strong()
                                            .color(theme::TEXT),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{}/{} belegt",
                                                    used, dev.led_count
                                                ))
                                                .size(11.0)
                                                .color(if used > dev.led_count {
                                                    theme::BAD
                                                } else {
                                                    theme::TEXT_FAINT
                                                }),
                                            );
                                            theme::pill(
                                                ui,
                                                format!("{} LEDs", dev.led_count),
                                                theme::TEXT_DIM,
                                                false,
                                            );
                                        },
                                    );
                                });
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(&dev.host)
                                            .size(11.5)
                                            .color(theme::TEXT_FAINT),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            egui::ComboBox::from_id_salt(format!("proto{i}"))
                                                .width(96.0)
                                                .selected_text(match dev.protocol {
                                                    Protocol::Ddp => "DDP",
                                                    Protocol::Drgb => "DRGB",
                                                })
                                                .show_ui(ui, |ui| {
                                                    ui.selectable_value(
                                                        &mut dev.protocol,
                                                        Protocol::Ddp,
                                                        "DDP · 4048",
                                                    ).hand_cursor();
                                                    ui.selectable_value(
                                                        &mut dev.protocol,
                                                        Protocol::Drgb,
                                                        "DRGB · 21324",
                                                    ).hand_cursor();
                                                }).response.hand_cursor();
                                        },
                                    );
                                });
                                ui.add_space(2.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("Helligkeit")
                                            .size(11.5)
                                            .color(theme::TEXT_FAINT),
                                    );
                                    ui.add(
                                        egui::Slider::new(&mut dev.brightness, 0.0..=1.0)
                                            .show_value(false),
                                    ).hand_cursor();
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{:.0} %",
                                            dev.brightness * 100.0
                                        ))
                                        .size(11.0)
                                        .color(theme::TEXT_DIM),
                                    );
                                });
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("max. FPS")
                                            .size(11.5)
                                            .color(theme::TEXT_FAINT),
                                    );
                                    let opts = [
                                        0u32, 15, 20, 24, 25, 30, 40, 50, 60, 72, 100, 120, 144,
                                    ];
                                    if theme::combo(
                                        ui,
                                        &format!("fps{i}"),
                                        92.0,
                                        &mut dev.max_fps,
                                        |v| {
                                            if v == 0 {
                                                "global".to_string()
                                            } else {
                                                format!("{v} fps")
                                            }
                                        },
                                        &opts,
                                    ) {
                                        self.sync_config();
                                        self.config_dirty = true;
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut dev.enabled, "aktiv").hand_cursor().on_hover_cursor(egui::CursorIcon::PointingHand);
                                    ui.checkbox(&mut dev.reverse, "Streifen gespiegelt").hand_cursor()
                                        .on_hover_text(
                                            "dreht den kompletten Streifen, wenn die Datenleitung am anderen Ende sitzt",
                                        );
                                    ui.checkbox(&mut dev.bgr, "BGR").hand_cursor()
                                        .on_hover_text("nur wenn Rot und Blau vertauscht sind");
                                });
                                ui.horizontal(|ui| {
                                    if ui.button("Aus").hand_cursor().clicked() {
                                        let mut sender =
                                            wled::Sender::new(&dev.host, dev.protocol);
                                        sender.send(&vec![
                                            [0u8, 0u8, 0u8];
                                            dev.led_count as usize
                                        ]);
                                    }
                                    if ui.button("Entfernen").hand_cursor().clicked() {
                                        remove = Some(i);
                                    }
                                });
                            });
                            ui.add_space(2.0);
                        }
                        if let Some(i) = remove {
                            let id = devices[i].id.clone();
                            devices.remove(i);
                            self.cfg.zones.retain(|z| z.device != id);
                            self.sync_config();
                            self.config_dirty = true;
                        }
                        let hosts: Vec<String> = devices.iter().map(|d| d.host.clone()).collect();
                        if hosts != self.last_hosts {
                            self.last_hosts = hosts;
                            self.sync_config();
                            self.config_dirty = true;
                        }
                        self.cfg.devices = devices;

                        self.discovery_rows(ui);
                    });
            });
    }

    fn discovery_rows(&mut self, ui: &mut egui::Ui) {
        let Some((found, done)) = self.scan.clone() else {
            return;
        };
        let list = found.lock().unwrap().clone();
        let pending: Vec<WledInfo> = list
            .iter()
            .filter(|i| self.cfg.device(&i.host).is_none())
            .cloned()
            .collect();
        let finished = *done.lock().unwrap();
        if pending.is_empty() {
            if finished {
                self.scan = None;
                return;
            }
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new("suche Geräte…")
                        .size(11.5)
                        .color(theme::TEXT_FAINT),
                );
            });
            ui.ctx().request_repaint_after(Duration::from_millis(250));
            return;
        }
        ui.add_space(2.0);
        theme::card(ui, |ui| {
            ui.label(
                egui::RichText::new(if finished {
                    "gefunden"
                } else {
                    "gefunden – Suche läuft weiter"
                })
                .size(10.5)
                .color(theme::TEXT_FAINT),
            );
            for info in &pending {
                ui.horizontal(|ui| {
                    theme::dot(ui, theme::ACCENT);
                    ui.label(
                        egui::RichText::new(format!("{} · {}", info.name, info.host))
                            .size(12.0),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if icons::button(ui, Icon::Plus, "hinzufügen") {
                                let (h, n, l) =
                                    (info.host.clone(), info.name.clone(), info.leds);
                                self.add_device(h, n, l);
                            }
                            theme::pill(
                                ui,
                                format!("{} LEDs", info.leds),
                                theme::TEXT_DIM,
                                false,
                            );
                            theme::pill(ui, format!("v{}", info.version), theme::TEXT_FAINT, false);
                            let _ = &info.arch;
                        },
                    );
                });
            }
        });
        if !finished {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }

    fn zones_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("zones")
            .resizable(true)
            .default_width(350.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        theme::section(ui, "Zonen");
                        let ranges = self.cfg.zone_ranges();
                        let mut zones = std::mem::take(&mut self.cfg.zones);
                        let mut remove: Option<usize> = None;
                        let mut moved: Option<(usize, isize)> = None;
                        for i in 0..zones.len() {
                            let (name, count, color, enabled, vertical, used) = {
                                let z = &zones[i];
                                (
                                    z.name.clone(),
                                    z.count,
                                    egui::Color32::from_rgb(z.color[0], z.color[1], z.color[2]),
                                    z.enabled,
                                    z.vertical,
                                    ranges.get(i).copied().unwrap_or((0, 0)),
                                )
                            };
                            let selected = self.selected == Some(i);
                            let frame = egui::Frame::new()
                                .fill(if selected { theme::CARD_HI } else { theme::CARD })
                                .stroke(egui::Stroke::new(
                                    1.0_f32,
                                    if selected { theme::ACCENT } else { theme::BORDER },
                                ))
                                .corner_radius(egui::CornerRadius::same(10))
                                .inner_margin(egui::Margin::symmetric(10, 7));
                            frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    theme::dot(ui, color);
                                    let mut label = egui::RichText::new(&name).size(13.0);
                                    if !enabled {
                                        label = label.color(theme::TEXT_FAINT);
                                    }
                                    if ui
                                        .add(egui::Label::new(label).sense(Sense::click()))
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        self.selected = Some(i);
                                    }
                                    let mut flags = Vec::new();
                                    if vertical {
                                        flags.push("vertikal");
                                    }
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{}–{} · {}{}",
                                            used.0,
                                            used.1.saturating_sub(1),
                                            count,
                                            if flags.is_empty() {
                                                String::new()
                                            } else {
                                                format!(" · {}", flags.join(", "))
                                            }
                                        ))
                                        .size(11.0)
                                        .color(theme::TEXT_FAINT),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if icons::button(ui, Icon::Close, "Zone löschen") {
                                                remove = Some(i);
                                            }
                                            if icons::button(ui, Icon::ChevronDown, "nach unten") {
                                                moved = Some((i, 1));
                                            }
                                            if icons::button(ui, Icon::ChevronUp, "nach oben") {
                                                moved = Some((i, -1));
                                            }
                                        },
                                    );
                                });
                            });
                            ui.add_space(2.0);
                        }
                        if let Some((i, d)) = moved {
                            let j = (i as isize + d).clamp(0, zones.len() as isize - 1) as usize;
                            if j != i {
                                zones.swap(i, j);
                                self.selected = Some(j);
                                self.sync_config();
                                self.config_dirty = true;
                            }
                        }
                        if let Some(i) = remove {
                            zones.remove(i);
                            self.selected = None;
                            self.sync_config();
                            self.config_dirty = true;
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if icons::labelled(ui, Icon::Plus, "Zone", true) {
                                self.add_free_zone();
                            }
                            if icons::labelled(ui, Icon::Split, "Aufteilen", false) {
                                self.split_into_edges();
                            }
                        });
                        if icons::labelled(ui, Icon::Grid, "Geräte auf Kanten verteilen", false) {
                            self.arrange_devices_on_edges();
                        }

                        if let Some(i) = self.selected {
                            if i < zones.len() {
                                ui.add_space(10.0);
                                theme::section(ui, "Zone bearbeiten");
                                let devices: Vec<(String, String)> = self
                                    .cfg
                                    .devices
                                    .iter()
                                    .map(|d| (d.id.clone(), d.name.clone()))
                                    .collect();
                                let cap = self.capture_dims();
                                let (sw, sh) = {
                                    let s = *self.shared.screen.lock().unwrap();
                                    if s.0 > 0 {
                                        s
                                    } else {
                                        (2560, 1440)
                                    }
                                };
                                theme::card(ui, |ui| {
                                    let z = &mut zones[i];
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut z.name)
                                                .desired_width(130.0),
                                        ).hand_cursor();
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let mut c = z.count as u32;
                                                if ui
                                                    .add(
                                                        egui::DragValue::new(&mut c)
                                                            .speed(1.0)
                                                            .range(1..=2000)
                                                            .suffix(" LEDs"),
                                                    )
                                                    .changed()
                                                {
                                                    z.count = c as u16;
                                                    self.sync_config();
                                                }
                                            },
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        egui::ComboBox::from_id_salt("zdev")
                                            .width(140.0)
                                            .selected_text(
                                                devices
                                                    .iter()
                                                    .find(|(id, _)| *id == z.device)
                                                    .map(|(_, n)| n.clone())
                                                    .unwrap_or_else(|| "—".into()),
                                            )
                                            .show_ui(ui, |ui| {
                                                for (id, name) in &devices {
                                                    ui.selectable_value(
                                                        &mut z.device,
                                                        id.clone(),
                                                        name.clone(),
                                                    ).hand_cursor();
                                                }
                                            }).response.hand_cursor();
                                        ui.checkbox(&mut z.enabled, "an").hand_cursor().on_hover_cursor(egui::CursorIcon::PointingHand);
                                        ui.checkbox(&mut z.vertical, "vertikal").hand_cursor().on_hover_cursor(egui::CursorIcon::PointingHand);
                                    });
                                    ui.add_space(2.0);
                                    let reversed = self
                                        .cfg
                                        .device(&z.device)
                                        .map(|d| d.reverse)
                                        .unwrap_or(false);
                                    theme::hint(
                                        ui,
                                        if reversed {
                                            "Kette läuft rückwärts – LED 0 liegt am anderen Ende"
                                        } else {
                                            "Richtung wird pro Gerät gesetzt: 'Kette läuft rückwärts'"
                                        },
                                    );
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("Ausschnitt")
                                                .size(11.5)
                                                .color(theme::TEXT_FAINT),
                                        );
                                        let mut pw = (z.w * cap.0 as f32).max(1.0);
                                        let mut ph = (z.h * cap.1 as f32).max(1.0);
                                        if ui
                                            .add(
                                                egui::DragValue::new(&mut pw)
                                                    .speed(1.0)
                                                    .range(1.0..=cap.0 as f32)
                                                    .suffix(" px breit"),
                                            )
                                            .changed()
                                        {
                                            z.w = (pw / cap.0 as f32).clamp(1.0 / cap.0 as f32, 1.0);
                                        }
                                        if ui
                                            .add(
                                                egui::DragValue::new(&mut ph)
                                                    .speed(1.0)
                                                    .range(1.0..=cap.1 as f32)
                                                    .suffix(" px hoch"),
                                            )
                                            .changed()
                                        {
                                            z.h = (ph / cap.1 as f32).clamp(1.0 / cap.1 as f32, 1.0);
                                        }
                                    });
                                    theme::hint(
                                        ui,
                                        &format!(
                                            "{:.0} × {:.0} Schirm-Pixel bei {}×{}",
                                            z.w * sw as f32,
                                            z.h * sh as f32,
                                            sw,
                                            sh
                                        ),
                                    );
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        if ui.button("Breite füllen").hand_cursor().clicked() {
                                            z.x = 0.0;
                                            z.w = 1.0;
                                        }
                                        if ui.button("Höhe füllen").hand_cursor().clicked() {
                                            z.y = 0.0;
                                            z.h = 1.0;
                                        }
                                        if ui.button("an Kante").hand_cursor().clicked() {
                                            snap_to_edge(z);
                                        }
                                    });
                                    ui.add_space(2.0);
                                    ui.add(
                                        egui::Slider::new(&mut z.gain, 0.0..=3.0)
                                            .text("Verstärkung"),
                                    ).hand_cursor();
                                });
                            }
                        }

                        ui.add_space(10.0);
                        egui::CollapsingHeader::new(
                            egui::RichText::new("Bild & Glättung")
                                .size(12.5)
                                .color(theme::TEXT_DIM),
                        )
                        .id_salt("advanced")
                        .default_open(false)
                        .show(ui, |ui| {
                            theme::card(ui, |ui| {
                                let st = &mut self.cfg.settings;
                                ui.add(
                                    egui::Slider::new(&mut st.smoothing_ms, 0.0..=600.0)
                                        .text("zeitliche Glättung")
                                        .suffix(" ms"),
                                ).hand_cursor();
                                ui.add(
                                    egui::Slider::new(&mut st.spatial, 0.0..=1.0)
                                        .text("räumliche Glättung"),
                                ).hand_cursor();
                                ui.add(
                                    egui::Slider::new(&mut st.slew, 0.0..=3000.0)
                                        .text("max. Änderung/s"),
                                ).hand_cursor();
                                ui.add(
                                    egui::Slider::new(&mut st.brightness, 0.0..=1.0)
                                        .text("Helligkeit"),
                                ).hand_cursor();
                                ui.add(egui::Slider::new(&mut st.gamma, 0.5..=2.5).text("Gamma")).hand_cursor();
                                ui.add(
                                    egui::Slider::new(&mut st.saturation, 0.0..=2.0)
                                        .text("Sättigung"),
                                ).hand_cursor();
                                ui.horizontal(|ui| {
                                    ui.label("Bildrate");
                                    let opts = [24u32, 25, 30, 40, 50, 60, 72, 100, 120, 144];
                                    theme::combo(
                                        ui,
                                        "globalfps",
                                        92.0,
                                        &mut st.target_fps,
                                        |v| format!("{v} fps"),
                                        &opts,
                                    );
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Aufnahme");
                                    let (sw, sh) = *self.shared.screen.lock().unwrap();
                                    let (sw, sh) = if sw > 0 { (sw, sh) } else { (2560, 1440) };
                                    let opts = [
                                        320u32, 480, 640, 960, 1280, 1600, 1920, 2560, 3840,
                                    ];
                                    let mut cw = st.capture_width;
                                    theme::combo(
                                        ui,
                                        "capturew",
                                        152.0,
                                        &mut cw,
                                        |w| {
                                            let h = (w as f32 * sh as f32 / sw as f32) as u32;
                                            if (w as f32 / sw as f32 - 1.0).abs() < 0.02 {
                                                format!("{w} × {h} (nativ)")
                                            } else {
                                                format!("{w} × {h}")
                                            }
                                        },
                                        &opts,
                                    );
                                    if cw != st.capture_width {
                                        st.capture_width = cw;
                                    }
                                });
                                theme::hint(
                                    ui,
                                    "höher = feinere Zonen, kostet linear CPU im Bildpfad",
                                );
                                ui.checkbox(&mut st.snap, "Snap beim Ziehen").hand_cursor().on_hover_cursor(egui::CursorIcon::PointingHand);
                                ui.checkbox(
                                    &mut st.pause_when_focused,
                                    "Pause, solange dieses Fenster aktiv ist",
                                ).hand_cursor()
                                .on_hover_text(
                                    "verhindert, dass sich das Fenster selbst aufnimmt",
                                );
                                self.config_dirty = true;
                            });
                        }).header_response.hand_cursor();
                        self.cfg.zones = zones;
                    });
            });
    }

    fn canvas_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(ctx, |ui| {
                let focused = ui.ctx().input(|i| i.focused);
                if focused {
                    let frame = self.shared.frame.lock().unwrap().clone();
                    self.update_texture(ui, Some(&frame));
                }
                let (w, h) = self.capture_dims();
                let aspect = w as f32 / h as f32;
                let avail = ui.available_rect_before_wrap();
                let view = View::new(avail.shrink(10.0), aspect);

                // matte behind the picture
                let painter = ui.painter_at(avail);
                painter.rect(
                    view.rect.expand(8.0),
                    egui::CornerRadius::same(14),
                    theme::PANEL,
                    egui::Stroke::new(1.0_f32, theme::BORDER),
                    egui::StrokeKind::Inside,
                );
                painter.rect_filled(view.rect, 4, egui::Color32::from_rgb(8, 9, 11));
                if let Some(t) = self.texture.as_ref() {
                    painter.image(
                        t.id(),
                        view.rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                painter.rect_stroke(
                    view.rect,
                    egui::CornerRadius::same(4),
                    egui::Stroke::new(1.0_f32, theme::BORDER),
                    egui::StrokeKind::Inside,
                );

                let resp = ui.allocate_rect(view.rect, Sense::click_and_drag());
                self.draw_zones(ui, &view, &resp);

                // status chip
                let status = self.show_status.clone();
                let (text, color) = match &status {
                    CaptureStatus::Running { w, h } => {
                        (format!("live · {w}×{h}"), theme::GOOD)
                    }
                    CaptureStatus::Starting => ("Aufnahme startet…".to_string(), theme::WARN),
                    CaptureStatus::Error(e) => (format!("Fehler: {e}"), theme::BAD),
                    CaptureStatus::Idle => ("warte auf Aufnahme".to_string(), theme::TEXT_DIM),
                };
                let chip_pos = Pos2::new(view.rect.left() + 8.0, view.rect.bottom() - 26.0);
                theme::chip(&ui.painter_at(view.rect), chip_pos, &text, color);

                if !focused {
                    theme::chip(
                        &ui.painter_at(view.rect),
                        Pos2::new(view.rect.right() - 190.0, view.rect.bottom() - 26.0),
                        "Anzeige eingefroren",
                        theme::TEXT_FAINT,
                    );
                }
            });
    }

    fn strip_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("out")
            .resizable(true)
            .default_height(132.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    theme::section(ui, "Ausgabe");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let status = self.show_status.clone();
                        let (txt, col) = match &status {
                            CaptureStatus::Running { w, h } => {
                                (format!("Aufnahme {w}×{h}"), theme::TEXT_DIM)
                            }
                            CaptureStatus::Starting => ("startet…".into(), theme::WARN),
                            CaptureStatus::Error(_) => ("Fehler".into(), theme::BAD),
                            CaptureStatus::Idle => ("warte auf Aufnahme".into(), theme::TEXT_DIM),
                        };
                        theme::pill(ui, txt, col, false);
                        theme::pill(
                            ui,
                            format!("{:.0} fps rein", self.show_fps_in),
                            theme::TEXT_DIM,
                            false,
                        );
                        theme::pill(
                            ui,
                            format!("{} µs", self.show_sample),
                            theme::TEXT_FAINT,
                            false,
                        );
                        if self.shared.paused.load(std::sync::atomic::Ordering::Relaxed) {
                            theme::pill(ui, "pausiert", theme::WARN, true);
                        }
                        if !self.status.is_empty() {
                            ui.label(
                                egui::RichText::new(&self.status)
                                    .size(11.0)
                                    .color(theme::TEXT_FAINT),
                            );
                        }
                    });
                });
                ui.add_space(2.0);
                let mut any = false;
                for (i, dev) in self.cfg.devices.iter().enumerate() {
                    if !dev.enabled {
                        continue;
                    }
                    any = true;
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&dev.name)
                                .size(11.5)
                                .color(theme::TEXT_DIM),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("{} LEDs", dev.led_count))
                                    .size(10.5)
                                    .color(theme::TEXT_FAINT),
                            );
                        });
                    });
                    let colors = self.show_last.get(i).cloned().unwrap_or_default();
                    theme::led_tape(ui, &colors, 22.0);
                    ui.add_space(2.0);
                }
                if !any {
                    theme::hint(ui, "noch kein Gerät aktiv");
                }
            });
    }
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let focused = ctx.input(|i| i.focused);
        let want_pause = self
            .tray_state
            .manual_pause
            .load(std::sync::atomic::Ordering::Relaxed)
            || (self.cfg.settings.pause_when_focused && focused);
        let was = self
            .shared
            .paused
            .swap(want_pause, std::sync::atomic::Ordering::Relaxed);
        if was != want_pause && std::env::var("AMBILIGHT_STATS").is_ok() {
            eprintln!(
                "pipeline {} (focused={focused})",
                if want_pause { "PAUSED" } else { "LIVE" }
            );
        }
        if focused {
            self.show_last = self.shared.last.lock().unwrap().clone();
            self.show_fps_in = *self.shared.fps_in.lock().unwrap();
            self.show_fps_out = *self.shared.fps_out.lock().unwrap();
            self.show_sample = *self.shared.sample_us.lock().unwrap();
            self.show_status = self.capture_status();
            self.show_dims = self.capture_dims();
        }
        // --- window lifecycle driven by the tray ---
        if self.tray_state.show.swap(false, std::sync::atomic::Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if self.tray_state.quit.load(std::sync::atomic::Ordering::Relaxed) && !self.quitting {
            self.quitting = true;
            // the pipeline lives in its own thread, so save and go
            self.cfg.save();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            // closing the window keeps the ambilight running: it just minimises
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            let manual = self
                .tray_state
                .manual_pause
                .load(std::sync::atomic::Ordering::Relaxed);
            self.shared
                .paused
                .store(manual, std::sync::atomic::Ordering::Relaxed);
        }
        if let Ok(mut st) = self.tray_state.status.lock() {
            *st = format!(
                "{} · {:.0} fps{}",
                match &self.show_status {
                    CaptureStatus::Running { w, h } => format!("live {w}×{h}"),
                    CaptureStatus::Starting => "startet…".to_string(),
                    CaptureStatus::Error(e) => format!("Fehler: {e}"),
                    CaptureStatus::Idle => "warte auf Aufnahme".to_string(),
                },
                self.show_fps_out,
                if self.shared.paused.load(std::sync::atomic::Ordering::Relaxed) {
                    " · pausiert"
                } else {
                    ""
                }
            );
        }
        self.keyboard(ctx);
        self.toolbar(ctx);
        self.devices_panel(ctx);
        self.zones_panel(ctx);
        self.strip_panel(ctx);
        self.canvas_panel(ctx);
        if self.pushed != self.cfg {
            self.shared.set_config(&self.cfg);
            self.pushed = self.cfg.clone();
            self.config_dirty = true;
        }
        self.save_if_due();
        if focused {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
