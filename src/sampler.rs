use crate::config::Config;

#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// All LED cells of one zone, in screen order (left->right or top->bottom).
#[derive(Debug, Clone)]
pub struct ZoneGroup {
    pub cells: Vec<(u16, Cell)>,
}

#[derive(Debug, Clone)]
pub struct DevicePlan {
    pub groups: Vec<ZoneGroup>,
    /// per led gain coming from the zone, index = led index
    pub gains: Vec<f32>,
    pub led_count: u16,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub w: u32,
    pub devices: Vec<DevicePlan>,
}

/// Turn the normalised zone layout into concrete screen cells for the current
/// capture resolution. Everything is normalised, so any resolution works.
pub fn build(cfg: &Config, w: u32, h: u32) -> Plan {
    let mut devices = Vec::new();
    for dev in cfg.devices.iter().filter(|d| d.enabled) {
        let mut groups: Vec<ZoneGroup> = Vec::new();
        let mut gains = vec![0.0f32; dev.led_count as usize];
        let mut used: u32 = 0;
        for zone in cfg
            .zones
            .iter()
            .filter(|z| z.enabled && z.device == dev.id)
        {
            let x0 = ((zone.x * w as f32).floor() as i64).clamp(0, w as i64 - 1) as u32;
            let y0 = ((zone.y * h as f32).floor() as i64).clamp(0, h as i64 - 1) as u32;
            let x1 =
                (((zone.x + zone.w) * w as f32).ceil() as i64).clamp(x0 as i64 + 1, w as i64) as u32;
            let y1 =
                (((zone.y + zone.h) * h as f32).ceil() as i64).clamp(y0 as i64 + 1, h as i64) as u32;
            let n = zone.count.max(1) as u32;
            let start = zone.offset.map(|o| o as u32).unwrap_or(used);
            used = start + n;
            let mut cells = Vec::with_capacity(n as usize);
            for i in 0..n {
                let cell = if zone.vertical {
                    let cy0 = y0 + (i * (y1 - y0)) / n;
                    let cy1 = (y0 + ((i + 1) * (y1 - y0)) / n).max(cy0 + 1).min(y1);
                    Cell {
                        x0,
                        y0: cy0,
                        x1,
                        y1: cy1,
                    }
                } else {
                    let cx0 = x0 + (i * (x1 - x0)) / n;
                    let cx1 = (x0 + ((i + 1) * (x1 - x0)) / n).max(cx0 + 1).min(x1);
                    Cell {
                        x0: cx0,
                        y0,
                        x1: cx1,
                        y1,
                    }
                };
                let led = start + i;
                if led < dev.led_count as u32 {
                    if let Some(g) = gains.get_mut(led as usize) {
                        *g = zone.gain;
                    }
                    cells.push((led as u16, cell));
                }
            }
            groups.push(ZoneGroup { cells });
        }
        devices.push(DevicePlan {
            groups,
            gains,
            led_count: dev.led_count,
        });
    }
    Plan { w, devices }
}

/// Mean colour of one cell. The cell is fully covered (with a stride only for
/// very large cells) - averaging the whole LED area is what removes flicker.
#[inline]
fn cell_color(frame: &[u8], frame_w: u32, cell: &Cell) -> [f32; 3] {
    let cw = (cell.x1 - cell.x0).max(1);
    let ch = (cell.y1 - cell.y0).max(1);
    let step_x = (cw / 24).max(1);
    let step_y = (ch / 24).max(1);
    let mut sum = [0u32; 3];
    let mut n = 0u32;
    let stride = frame_w as usize * 3;
    let mut y = cell.y0;
    while y < cell.y1 {
        let row = y as usize * stride;
        let mut x = cell.x0;
        while x < cell.x1 {
            let o = row + x as usize * 3;
            if o + 2 < frame.len() {
                sum[0] += frame[o] as u32;
                sum[1] += frame[o + 1] as u32;
                sum[2] += frame[o + 2] as u32;
                n += 1;
            }
            x += step_x;
        }
        y += step_y;
    }
    if n == 0 {
        [0.0, 0.0, 0.0]
    } else {
        [
            sum[0] as f32 / n as f32,
            sum[1] as f32 / n as f32,
            sum[2] as f32 / n as f32,
        ]
    }
}

/// Full LED buffer for every device, already in physical LED order.
/// `spatial` (0..1) blurs along the strip, which makes neighbouring LEDs share
/// their colour instead of each one hopping on its own.
pub fn sample(plan: &Plan, frame: &[u8], spatial: f32, out: &mut Vec<Vec<[f32; 3]>>) {
    out.clear();
    for dev in &plan.devices {
        let mut buf = vec![[0f32; 3]; dev.led_count as usize];
        for group in &dev.groups {
            if group.cells.is_empty() {
                continue;
            }
            let mut cols: Vec<[f32; 3]> = group
                .cells
                .iter()
                .map(|(_, c)| cell_color(frame, plan.w, c))
                .collect();
            if spatial > 0.001 && cols.len() > 2 {
                let src = cols.clone();
                for i in 0..cols.len() {
                    let a = src[i.saturating_sub(1)];
                    let b = src[i];
                    let c = src[(i + 1).min(src.len() - 1)];
                    for k in 0..3 {
                        let soft = (a[k] + 2.0 * b[k] + c[k]) * 0.25;
                        cols[i][k] = b[k] + (soft - b[k]) * spatial;
                    }
                }
            }
            for (k, (idx, _)) in group.cells.iter().enumerate() {
                if let Some(slot) = buf.get_mut(*idx as usize) {
                    *slot = cols[k];
                }
            }
        }
        out.push(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DeviceCfg, Zone};

    fn cfg_with(zone: Zone) -> Config {
        let mut cfg = Config::default();
        cfg.devices.push(DeviceCfg::new("127.0.0.1", "test", 4));
        let mut z = zone;
        z.device = cfg.devices[0].id.clone();
        z.count = 4;
        cfg.zones.push(z);
        cfg
    }

    fn zone() -> Zone {
        let mut z = Zone::preset(crate::config::ZonePreset::Full, String::new(), 4, [0, 0, 0]);
        z.x = 0.0;
        z.y = 0.0;
        z.w = 1.0;
        z.h = 1.0;
        z
    }

    /// left half white, right half black
    fn frame_lr(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w / 2 {
                let o = ((y * w + x) * 3) as usize;
                v[o] = 255;
                v[o + 1] = 255;
                v[o + 2] = 255;
            }
        }
        v
    }

    /// top half white, bottom half black
    fn frame_tb(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 3) as usize];
        for y in 0..h / 2 {
            for x in 0..w {
                let o = ((y * w + x) * 3) as usize;
                v[o] = 255;
                v[o + 1] = 255;
                v[o + 2] = 255;
            }
        }
        v
    }

    #[test]
    fn horizontal_zone_reads_left_to_right() {
        let cfg = cfg_with(zone());
        let plan = build(&cfg, 100, 10);
        let mut out = Vec::new();
        sample(&plan, &frame_lr(100, 10), 0.0, &mut out);
        assert_eq!(out[0][0][0] as u8, 255, "LED 0 must be white (left half)");
        assert_eq!(out[0][3][0] as u8, 0, "LED 3 must be black (right half)");
    }

    #[test]
    fn vertical_zone_reads_top_to_bottom() {
        let mut z = zone();
        z.vertical = true;
        let cfg = cfg_with(z);
        let plan = build(&cfg, 10, 100);
        let mut out = Vec::new();
        sample(&plan, &frame_tb(10, 100), 0.0, &mut out);
        assert_eq!(out[0][0][0] as u8, 255, "LED 0 white = top half");
        assert_eq!(out[0][3][0] as u8, 0, "LED 3 black = bottom half");
    }
}
