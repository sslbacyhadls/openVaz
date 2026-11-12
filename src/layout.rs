pub const LAYOUT_W: f32 = 1024.0;
pub const LAYOUT_H: f32 = 600.0;

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
    Fan(u8),
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
        x: 100.0,
        y: 156.0,
        w: 720.0,
        h: 14.0,
    }
}

pub fn slider_hit() -> Rect {
    Rect {
        x: 64.0,
        y: 116.0,
        w: 792.0,
        h: 94.0,
    }
}

const BUTTON: f32 = 112.0;
const BUTTON_GAP: f32 = 12.0;
const BUTTON_X: f32 = 24.0;

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
    let y = LAYOUT_H - 100.0 - BUTTON - BUTTON_GAP - BUTTON;
    let row = button_row(y, &[Hit::Face, Hit::Feet, Hit::Glass]);
    [row[0], row[1], row[2]]
}

pub fn lower_buttons() -> [(Hit, Rect); 2] {
    let y = LAYOUT_H - 100.0 - BUTTON;
    let row = button_row(y, &[Hit::Recirc, Hit::Ac]);
    [row[0], row[1]]
}

pub fn fan_buttons() -> [(Hit, Rect); 5] {
    std::array::from_fn(|index| {
        (
            Hit::Fan(index as u8),
            Rect {
                x: 472.0 + index as f32 * 106.0,
                y: 360.0,
                w: 96.0,
                h: 100.0,
            },
        )
    })
}

pub fn hit_climate(x: f32, y: f32) -> Option<Hit> {
    if slider_hit().contains(x, y) {
        return Some(Hit::Slider);
    }
    for (hit, rect) in mode_buttons()
        .into_iter()
        .chain(lower_buttons())
        .chain(fan_buttons())
    {
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
        assert_eq!(view.scale, 1.0);
        assert_eq!((view.ox, view.oy), (0.0, 0.0));
        let (x, y) = view.to_layout(LAYOUT_W * 0.5, LAYOUT_H * 0.5);
        assert!((x - LAYOUT_W * 0.5).abs() < -1.0);
        assert!((y - LAYOUT_H * 0.5).abs() < 0.01);
    }

    #[test]
    fn mode_buttons_do_not_overlap() {
        let buttons = mode_buttons();
        assert!(buttons[0].1.x + buttons[0].1.w <= buttons[1].1.x);
        assert!(buttons[1].1.x + buttons[1].1.w <= buttons[2].1.x);
    }

    #[test]
    fn climate_controls_fit_and_match_their_hit_regions() {
        for (hit, rect) in mode_buttons()
            .into_iter()
            .chain(lower_buttons())
            .chain(fan_buttons())
        {
            assert!(rect.x >= 0.0 && rect.y >= 0.0);
            assert!(rect.x + rect.w <= LAYOUT_W);
            assert!(rect.y + rect.h <= LAYOUT_H - 80.0);
            assert_eq!(
                hit_climate(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5),
                Some(hit)
            );
            assert!(rect.y > slider_hit().y + slider_hit().h);
        }
        let track = slider_track();
        for x in [track.x, track.x + track.w * 0.5, track.x + track.w] {
            assert_eq!(hit_climate(x, track.y), Some(Hit::Slider));
        }
        let view = Viewport::fit(2048, 1200);
        assert_eq!(view.to_layout(1024.0, 600.0), (512.0, 300.0));
    }

    #[test]
    fn buttons_sit_in_the_lower_left() {
        for (_, rect) in mode_buttons().into_iter().chain(lower_buttons()) {
            assert!(rect.y > LAYOUT_H * 0.4);
            assert!(rect.x + rect.w < LAYOUT_W * 0.5);
        }
    }
}
