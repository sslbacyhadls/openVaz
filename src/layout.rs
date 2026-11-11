pub const LAYOUT_W: f32 = 1920.0;
pub const LAYOUT_H: f32 = 1080.0;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Face,
    Feet,
    Glass,
    Recirc,
    Ac,
    Slider,
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub scale: f32,
    pub ox: f32,
    pub oy: f32,
}

impl Viewport {
    pub fn fit(dst_w: u32, dst_h: u32) -> Self {
        let dw = dst_w.max(1) as f32;
        let dh = dst_h.max(1) as f32;
        let scale = (dw / LAYOUT_W).min(dh / LAYOUT_H);
        Self {
            scale,
            ox: (dw - LAYOUT_W * scale) * 0.5,
            oy: (dh - LAYOUT_H * scale) * 0.5,
        }
    }

    pub fn to_layout(self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.ox) / self.scale, (y - self.oy) / self.scale)
    }
}

pub fn slider_track() -> Rect {
    Rect {
        x: 196.0,
        y: 188.0,
        w: 720.0,
        h: 14.0,
    }
}

pub fn slider_hit() -> Rect {
    Rect {
        x: 160.0,
        y: 140.0,
        w: 792.0,
        h: 110.0,
    }
}

const BUTTON: f32 = 164.0;
const BUTTON_GAP: f32 = 16.0;
const BUTTON_X: f32 = 48.0;

fn button_row(y: f32, hits: &[Hit]) -> Vec<(Hit, Rect)> {
    hits.iter()
        .enumerate()
        .map(|(index, hit)| {
            (
                *hit,
                Rect {
                    x: BUTTON_X + index as f32 * (BUTTON + BUTTON_GAP),
                    y,
                    w: BUTTON,
                    h: BUTTON,
                },
            )
        })
        .collect()
}

pub fn mode_buttons() -> [(Hit, Rect); 3] {
    let y = LAYOUT_H - 112.0 - BUTTON - BUTTON_GAP - BUTTON;
    let row = button_row(y, &[Hit::Face, Hit::Feet, Hit::Glass]);
    [row[0], row[1], row[2]]
}

pub fn lower_buttons() -> [(Hit, Rect); 2] {
    let y = LAYOUT_H - 112.0 - BUTTON;
    let row = button_row(y, &[Hit::Recirc, Hit::Ac]);
    [row[0], row[1]]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveHit {
    Gas,
    Brake,
    Clutch,
    Gear(i32),
    Ignition,
    Starter,
}

pub fn drive_pedals() -> [(DriveHit, Rect); 3] {
    let y = 250.0;
    let h = 460.0;
    let w = 150.0;
    [
        (DriveHit::Clutch, Rect { x: 1288.0, y, w, h }),
        (DriveHit::Brake, Rect { x: 1462.0, y, w, h }),
        (DriveHit::Gas, Rect { x: 1636.0, y, w, h }),
    ]
}

pub fn drive_gears() -> [(DriveHit, Rect); 7] {
    let keys = [
        DriveHit::Gear(1),
        DriveHit::Gear(2),
        DriveHit::Gear(3),
        DriveHit::Gear(4),
        DriveHit::Gear(5),
        DriveHit::Gear(0),
        DriveHit::Gear(-1),
    ];
    let mut row = [(DriveHit::Gear(0), Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }); 7];
    for (index, hit) in keys.into_iter().enumerate() {
        row[index] = (
            hit,
            Rect {
                x: 80.0 + index as f32 * 112.0,
                y: 860.0,
                w: 100.0,
                h: 72.0,
            },
        );
    }
    row
}

pub fn hit_drive(x: f32, y: f32) -> Option<DriveHit> {
    for (hit, rect) in drive_pedals() {
        if rect.contains(x, y) {
            return Some(hit);
        }
    }
    for (hit, rect) in drive_gears() {
        if rect.contains(x, y) {
            return Some(hit);
        }
    }
    let ignition = Rect { x: 900.0, y: 860.0, w: 160.0, h: 72.0 };
    let starter = Rect { x: 1080.0, y: 860.0, w: 160.0, h: 72.0 };
    if ignition.contains(x, y) {
        return Some(DriveHit::Ignition);
    }
    if starter.contains(x, y) {
        return Some(DriveHit::Starter);
    }
    None
}

pub fn hit_climate(x: f32, y: f32) -> Option<Hit> {
    if slider_hit().contains(x, y) {
        return Some(Hit::Slider);
    }
    for (hit, rect) in mode_buttons().into_iter().chain(lower_buttons()) {
        if rect.contains(x, y) {
            return Some(hit);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_maps_the_center() {
        let view = Viewport::fit(LAYOUT_W as u32, LAYOUT_H as u32);
        let (x, y) = view.to_layout(LAYOUT_W * 0.5, LAYOUT_H * 0.5);
        assert!((x - LAYOUT_W * 0.5).abs() < 0.01);
        assert!((y - LAYOUT_H * 0.5).abs() < 0.01);
    }

    #[test]
    fn mode_buttons_do_not_overlap() {
        let buttons = mode_buttons();
        assert!(buttons[0].1.x + buttons[0].1.w <= buttons[1].1.x);
        assert!(buttons[1].1.x + buttons[1].1.w <= buttons[2].1.x);
    }

    #[test]
    fn buttons_sit_in_the_lower_left() {
        for (_, rect) in mode_buttons().into_iter().chain(lower_buttons()) {
            assert!(rect.y > LAYOUT_H * 0.5);
            assert!(rect.x + rect.w < LAYOUT_W * 0.5);
        }
    }
}
