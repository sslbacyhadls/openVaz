use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use fontdue::{Font, FontSettings};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::layout::{self, Hit, Rect};
use crate::model::{
    celsius_from_unit, Climate, EcuView, Screen, MIN_FAN, PROFILE_TITLE, TEMP_MAX_C, TEMP_MIN_C,
};

pub struct Typeface {
    regular: Font,
    bold: Font,
    cache: HashMap<(char, u16, bool), Glyph>,
}

struct Glyph {
    advance: f32,
    xmin: i32,
    ymin: i32,
    width: i32,
    height: i32,
    bitmap: Vec<u8>,
}

impl Typeface {
    pub fn load() -> Self {
        let regular = Font::from_bytes(
            include_bytes!("../assets/DejaVuSans.ttf").as_slice(),
            FontSettings::default(),
        )
        .expect("DejaVu Sans");
        let bold = Font::from_bytes(
            include_bytes!("../assets/DejaVuSans-Bold.ttf").as_slice(),
            FontSettings::default(),
        )
        .expect("DejaVu Sans Bold");
        Self {
            regular,
            bold,
            cache: HashMap::new(),
        }
    }

    fn glyph(&mut self, ch: char, size: f32, bold: bool) -> &Glyph {
        let key = (ch, size.round() as u16, bold);
        if !self.cache.contains_key(&key) {
            let font = if bold { &self.bold } else { &self.regular };
            let (metrics, bitmap) = font.rasterize(ch, size);
            self.cache.insert(
                key,
                Glyph {
                    advance: metrics.advance_width,
                    xmin: metrics.xmin,
                    ymin: metrics.ymin,
                    width: metrics.width as i32,
                    height: metrics.height as i32,
                    bitmap,
                },
            );
        }
        &self.cache[&key]
    }

    pub fn width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        text.chars()
            .map(|ch| self.glyph(ch, size, bold).advance)
            .sum()
    }
}

pub fn render(screen: Screen, climate: &Climate, ecu: &EcuView, fonts: &mut Typeface) -> Pixmap {
    let mut pixmap = Pixmap::new(layout::LAYOUT_W as u32, layout::LAYOUT_H as u32).expect("pixmap");
    pixmap.fill(color(0x12, 0x16, 0x1c));
    match screen {
        Screen::Climate => draw_climate(&mut pixmap, fonts, climate),
        Screen::Ecu => draw_ecu(&mut pixmap, fonts, ecu),
        Screen::Readings => draw_readings(&mut pixmap, fonts, ecu),
    }
    draw_dots(&mut pixmap, screen);
    pixmap
}

pub fn write_ppm(path: &Path, pixmap: &Pixmap) -> io::Result<()> {
    let mut file = File::create(path)?;
    write!(file, "P6\n{} {}\n255\n", pixmap.width(), pixmap.height())?;
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        file.write_all(&[color.red(), color.green(), color.blue()])?;
    }
    Ok(())
}

fn draw_climate(pixmap: &mut Pixmap, fonts: &mut Typeface, climate: &Climate) {
    text(pixmap, fonts, 24.0, 64.0, 36.0, true, color(0xf4, 0xf1, 0xea), "Климат");
    let profile_w = fonts.width(PROFILE_TITLE, 24.0, false);
    text(
        pixmap,
        fonts,
        layout::LAYOUT_W - 24.0 - profile_w,
        58.0,
        24.0,
        false,
        color(0x9a, 0xa3, 0xad),
        PROFILE_TITLE,
    );

    let track = layout::slider_track();
    let cold = format!("{TEMP_MIN_C}°");
    let hot = format!("{TEMP_MAX_C}°");
    text(pixmap, fonts, 24.0, track.y + 16.0, 24.0, false, color(0x7e, 0xb6, 0xd9), &cold);
    text(
        pixmap,
        fonts,
        track.x + track.w + 28.0,
        track.y + 16.0,
        24.0,
        false,
        color(0xe0, 0x70, 0x50),
        &hot,
    );

    fill_rect(pixmap, track, 7.0, color(0x3c, 0x46, 0x54));
    let knob_x = track.x + track.w * climate.temperature;
    fill_rect(
        pixmap,
        Rect {
            x: track.x,
            y: track.y,
            w: knob_x - track.x,
            h: track.h,
        },
        7.0,
        color(0xe0, 0x70, 0x50),
    );
    fill_circle(pixmap, knob_x, track.y + track.h * 0.5, 20.0, color(0xf4, 0xf1, 0xea));
    let degrees = format!("{}°", celsius_from_unit(climate.temperature));
    let degrees_w = fonts.width(&degrees, 22.0, true);
    text(
        pixmap,
        fonts,
        knob_x - degrees_w * 0.5,
        track.y - 16.0,
        22.0,
        true,
        color(0xf4, 0xf1, 0xea),
        &degrees,
    );

    for (hit, rect) in layout::mode_buttons() {
        let on = match hit {
            Hit::Face => climate.face,
            Hit::Feet => climate.feet,
            Hit::Glass => climate.glass,
            _ => false,
        };
        button(pixmap, rect, on, color(0xe8, 0xa3, 0x17), hit);
    }
    for (hit, rect) in layout::lower_buttons() {
        let (on, active) = match hit {
            Hit::Recirc => (climate.recirculation, color(0xe8, 0xa3, 0x17)),
            Hit::Ac => (climate.air_conditioner, color(0x3d, 0x9e, 0xe0)),
            _ => continue,
        };
        button(pixmap, rect, on, active, hit);
    }
    text(pixmap, fonts, 472.0, 324.0, 26.0, false, color(0x9a, 0xa3, 0xad), "Вентилятор");
    for (hit, rect) in layout::fan_buttons() {
        let Hit::Fan(speed) = hit else { continue };
        let on = (climate.fan - speed as f32 * MIN_FAN).abs() < 0.001;
        let (fill, ink) = if on {
            (color(0xe8, 0xa3, 0x17), color(0x12, 0x16, 0x1c))
        } else {
            (color(0x1c, 0x24, 0x30), color(0xf4, 0xf1, 0xea))
        };
        fill_rect(pixmap, rect, 16.0, fill);
        let label = if speed == 0 { "Выкл".to_string() } else { speed.to_string() };
        let size = if speed == 0 { 24.0 } else { 32.0 };
        let width = fonts.width(&label, size, true);
        text(pixmap, fonts, rect.x + (rect.w - width) * 0.5, rect.y + 60.0, size, true, ink, &label);
    }
    hint(pixmap, fonts, "Проведите пальцем влево — показания ЭБУ");
}

fn draw_ecu(pixmap: &mut Pixmap, fonts: &mut Typeface, ecu: &EcuView) {
    text(pixmap, fonts, 24.0, 64.0, 36.0, true, color(0xf4, 0xf1, 0xea), "ЭБУ");
    let (coolant, coolant_color) = match ecu.coolant_c {
        Some(degrees) if ecu.linked && degrees >= 105 => (
            format!("{degrees} °C"),
            color(0xd4, 0x55, 0x4a),
        ),
        Some(degrees) if ecu.linked => (format!("{degrees} °C"), color(0xf4, 0xf1, 0xea)),
        _ => ("нет данных".to_string(), color(0x9a, 0xa3, 0xad)),
    };
    let (errors, error_color) = if !ecu.linked {
        ("нет связи с ЭБУ".to_string(), color(0xd4, 0x55, 0x4a))
    } else if ecu.errors.is_empty() {
        ("нет".to_string(), color(0x9a, 0xa3, 0xad))
    } else {
        (ecu.errors.join(" "), color(0xd4, 0x55, 0x4a))
    };
    card(
        pixmap,
        fonts,
        24.0,
        112.0,
        "Температура охлаждающей жидкости",
        &coolant,
        coolant_color,
    );
    card(pixmap, fonts, 24.0, 284.0, "Коды ошибок", &errors, error_color);
    if ecu.linked {
        let clutch = match ecu.clutch {
            Some(true) => "Муфта кондиционера включена",
            Some(false) => "Муфта кондиционера выключена",
            None => "",
        };
        if !clutch.is_empty() {
            text(
                pixmap,
                fonts,
                24.0,
                482.0,
                28.0,
                false,
                color(0xf4, 0xf1, 0xea),
                clutch,
            );
        }
    }
    hint(pixmap, fonts, "Проведите пальцем влево — показания");
}

fn draw_readings(pixmap: &mut Pixmap, fonts: &mut Typeface, ecu: &EcuView) {
    text(
        pixmap,
        fonts,
        24.0,
        64.0,
        36.0,
        true,
        color(0xf4, 0xf1, 0xea),
        "Показания",
    );
    if !ecu.linked {
        let label = "нет связи с ЭБУ";
        let size = 36.0;
        let width = fonts.width(label, size, true);
        text(
            pixmap,
            fonts,
            (layout::LAYOUT_W - width) * 0.5,
            300.0,
            size,
            true,
            color(0xd4, 0x55, 0x4a),
            label,
        );
        hint(pixmap, fonts, "Проведите пальцем вправо — ЭБУ");
        return;
    }
    if let Some(heading) = ecu.readings_heading() {
        let width = fonts.width(&heading, 22.0, false);
        text(
            pixmap,
            fonts,
            layout::LAYOUT_W - 24.0 - width,
            58.0,
            22.0,
            false,
            color(0x9a, 0xa3, 0xad),
            &heading,
        );
    }
    let Some(rows) = ecu.reading_rows() else {
        text(
            pixmap,
            fonts,
            24.0,
            160.0,
            28.0,
            false,
            color(0x9a, 0xa3, 0xad),
            "нет данных",
        );
        hint(pixmap, fonts, "Проведите пальцем влево — климат");
        return;
    };
    let columns = 2;
    let per_column = rows.len().div_ceil(columns);
    let row_h = 52.0;
    let top = 108.0;
    let col_w = 488.0;
    for (index, (title, value)) in rows.iter().enumerate() {
        let column = index / per_column;
        let row = index % per_column;
        let x = 24.0 + column as f32 * (col_w + 16.0);
        let y = top + row as f32 * row_h;
        fill_rect(
            pixmap,
            Rect {
                x,
                y: y + 8.0,
                w: col_w,
                h: 1.0,
            },
            0.0,
            color(0x2a, 0x34, 0x42),
        );
        let value_w = fonts.width(value, 22.0, true);
        let title_room = (col_w - value_w - 16.0).max(40.0);
        let title = fit_label(fonts, title, 18.0, title_room);
        text(
            pixmap,
            fonts,
            x,
            y + 36.0,
            18.0,
            false,
            color(0x9a, 0xa3, 0xad),
            &title,
        );
        let ink = if value == "нет данных" {
            color(0x9a, 0xa3, 0xad)
        } else {
            color(0xf4, 0xf1, 0xea)
        };
        text(
            pixmap,
            fonts,
            x + col_w - value_w,
            y + 36.0,
            22.0,
            true,
            ink,
            value,
        );
    }
    hint(pixmap, fonts, "Проведите пальцем влево — климат");
}

fn card(pixmap: &mut Pixmap, fonts: &mut Typeface, x: f32, y: f32, title: &str, value: &str, value_color: Color) {
    let rect = Rect {
        x,
        y,
        w: layout::LAYOUT_W - 48.0,
        h: 144.0,
    };
    fill_rect(pixmap, rect, 18.0, color(0x1c, 0x24, 0x30));
    text(pixmap, fonts, x + 24.0, y + 44.0, 26.0, false, color(0x9a, 0xa3, 0xad), title);
    text(pixmap, fonts, x + 24.0, y + 108.0, 36.0, true, value_color, value);
}

fn button(pixmap: &mut Pixmap, rect: Rect, on: bool, active: Color, hit: Hit) {
    let fill = if on { active } else { color(0x1c, 0x24, 0x30) };
    let ink = if on { color(0x12, 0x16, 0x1c) } else { color(0xf4, 0xf1, 0xea) };
    fill_rect(pixmap, rect, 22.0, fill);
    let size = 62.0;
    let origin = (rect.x + (rect.w - size) * 0.5, rect.y + (rect.h - size) * 0.5);
    match hit {
        Hit::Face => icon_face(pixmap, origin, size, ink),
        Hit::Feet => icon_feet(pixmap, origin, size, ink),
        Hit::Glass => icon_glass(pixmap, origin, size, ink),
        Hit::Recirc => icon_recirc(pixmap, origin, size, ink),
        Hit::Ac => icon_snowflake(pixmap, origin, size, ink),
        Hit::Slider | Hit::Fan(_) => {}
    }
}

fn icon_face(pixmap: &mut Pixmap, origin: (f32, f32), size: f32, ink: Color) {
    let (x, y) = origin;
    let s = size / 78.0;
    let px = |u: f32, v: f32| (x + u * s, y + v * s);
    for (row, length) in [(18.0, 30.0), (36.0, 22.0), (54.0, 30.0)] {
        let start = px(4.0, row);
        let end = px(4.0 + length, row);
        thick_line(pixmap, start, end, 4.0 * s, ink);
        arrow_head(pixmap, px(8.0 + length, row), (1.0, 0.0), 11.0 * s, ink);
    }
    let (hx, hy) = px(56.0, 30.0);
    fill_circle(pixmap, hx, hy, 12.0 * s, ink);
    fill_rect(
        pixmap,
        Rect {
            x: hx - 16.0 * s,
            y: hy + 12.0 * s,
            w: 32.0 * s,
            h: 16.0 * s,
        },
        8.0 * s,
        ink,
    );
}

fn icon_feet(pixmap: &mut Pixmap, origin: (f32, f32), size: f32, ink: Color) {
    let (x, y) = origin;
    let s = size / 78.0;
    let px = |u: f32, v: f32| (x + u * s, y + v * s);
    for column in [18.0, 39.0, 60.0] {
        thick_line(pixmap, px(column, 6.0), px(column, 30.0), 4.0 * s, ink);
        arrow_head(pixmap, px(column, 42.0), (0.0, 1.0), 12.0 * s, ink);
    }
    for foot in [8.0, 42.0] {
        fill_rect(
            pixmap,
            Rect {
                x: x + foot * s,
                y: y + 52.0 * s,
                w: 28.0 * s,
                h: 16.0 * s,
            },
            8.0 * s,
            ink,
        );
    }
}

fn icon_glass(pixmap: &mut Pixmap, origin: (f32, f32), size: f32, ink: Color) {
    let (x, y) = origin;
    let s = size / 78.0;
    let px = |u: f32, v: f32| (x + u * s, y + v * s);
    let outer = [px(16.0, 8.0), px(62.0, 8.0), px(72.0, 68.0), px(6.0, 68.0)];
    let inner = [px(24.0, 16.0), px(54.0, 16.0), px(62.0, 60.0), px(16.0, 60.0)];
    fill_even_odd(pixmap, &[outer, inner], ink);
    for row in [30.0, 42.0, 54.0] {
        wave(pixmap, px(22.0, row), px(56.0, row), 3.5 * s, 3.2 * s, ink);
    }
}

fn icon_recirc(pixmap: &mut Pixmap, origin: (f32, f32), size: f32, ink: Color) {
    let (x, y) = origin;
    let s = size / 78.0;
    let center = (x + 39.0 * s, y + 40.0 * s);
    let start = -0.4_f32;
    let end = 5.2_f32;
    ring_arc(pixmap, center, 24.0 * s, 17.0 * s, start, end, ink);
    let tip = (
        center.0 + 20.5 * s * end.cos(),
        center.1 + 20.5 * s * end.sin(),
    );
    arrow_head(pixmap, tip, (-end.sin(), end.cos()), 14.0 * s, ink);
}

fn icon_snowflake(pixmap: &mut Pixmap, origin: (f32, f32), size: f32, ink: Color) {
    let (x, y) = origin;
    let s = size / 78.0;
    let center = (x + 39.0 * s, y + 39.0 * s);
    for index in 0..6 {
        let angle = index as f32 * std::f32::consts::FRAC_PI_3;
        let dir = (angle.cos(), angle.sin());
        let reach = 30.0 * s;
        thick_line(
            pixmap,
            center,
            (center.0 + dir.0 * reach, center.1 + dir.1 * reach),
            3.6 * s,
            ink,
        );
        let base = (center.0 + dir.0 * 16.0 * s, center.1 + dir.1 * 16.0 * s);
        for turn in [-0.75_f32, 0.75] {
            let branch = angle + turn;
            thick_line(
                pixmap,
                base,
                (base.0 + branch.cos() * 10.0 * s, base.1 + branch.sin() * 10.0 * s),
                3.2 * s,
                ink,
            );
        }
    }
}

fn fit_label(fonts: &mut Typeface, text: &str, size: f32, max_width: f32) -> String {
    if fonts.width(text, size, false) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut count = chars.len();
    while count > 1 {
        count -= 1;
        let mut cut: String = chars.iter().take(count).collect();
        cut.push('…');
        if fonts.width(&cut, size, false) <= max_width {
            return cut;
        }
    }
    "…".to_string()
}

fn hint(pixmap: &mut Pixmap, fonts: &mut Typeface, label: &str) {
    let size = 22.0;
    let width = fonts.width(label, size, false);
    text(
        pixmap,
        fonts,
        (layout::LAYOUT_W - width) * 0.5,
        layout::LAYOUT_H - 52.0,
        size,
        false,
        color(0x9a, 0xa3, 0xad),
        label,
    );
}

fn draw_dots(pixmap: &mut Pixmap, screen: Screen) {
    let y = layout::LAYOUT_H - 20.0;
    let active = color(0xf4, 0xf1, 0xea);
    let idle = color(0x3c, 0x46, 0x54);
    let order = [Screen::Climate, Screen::Ecu, Screen::Readings];
    let step = 28.0;
    let origin = layout::LAYOUT_W * 0.5 - step;
    for (index, item) in order.iter().enumerate() {
        fill_circle(
            pixmap,
            origin + index as f32 * step,
            y,
            6.0,
            if screen == *item { active } else { idle },
        );
    }
}

fn text(pixmap: &mut Pixmap, fonts: &mut Typeface, x: f32, baseline: f32, size: f32, bold: bool, color: Color, label: &str) {
    let mut cursor = x;
    for ch in label.chars() {
        let glyph = fonts.glyph(ch, size, bold);
        blit(
            pixmap,
            cursor + glyph.xmin as f32,
            baseline - glyph.height as f32 - glyph.ymin as f32,
            glyph.width,
            glyph.height,
            &glyph.bitmap,
            color,
        );
        cursor += glyph.advance;
    }
}

fn blit(pixmap: &mut Pixmap, x: f32, y: f32, width: i32, height: i32, bitmap: &[u8], color: Color) {
    let r = (color.red() * 255.0).round() as u32;
    let g = (color.green() * 255.0).round() as u32;
    let b = (color.blue() * 255.0).round() as u32;
    let stride = pixmap.width() as i32;
    let max_h = pixmap.height() as i32;
    let pixels = pixmap.pixels_mut();
    for row in 0..height {
        for col in 0..width {
            let coverage = bitmap[(row * width + col) as usize] as u32;
            if coverage == 0 {
                continue;
            }
            let px = x as i32 + col;
            let py = y as i32 + row;
            if px < 0 || py < 0 || px >= stride || py >= max_h {
                continue;
            }
            let index = (py * stride + px) as usize;
            let dst = pixels[index].demultiply();
            let inv = 255 - coverage;
            let nr = ((dst.red() as u32 * inv + r * coverage) / 255) as u8;
            let ng = ((dst.green() as u32 * inv + g * coverage) / 255) as u8;
            let nb = ((dst.blue() as u32 * inv + b * coverage) / 255) as u8;
            pixels[index] = tiny_skia::ColorU8::from_rgba(nr, ng, nb, 255).premultiply();
        }
    }
}

fn thick_line(pixmap: &mut Pixmap, start: (f32, f32), end: (f32, f32), thickness: f32, ink: Color) {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let nx = -dy / len * thickness * 0.5;
    let ny = dx / len * thickness * 0.5;
    fill_polygon(
        pixmap,
        &[
            (start.0 + nx, start.1 + ny),
            (end.0 + nx, end.1 + ny),
            (end.0 - nx, end.1 - ny),
            (start.0 - nx, start.1 - ny),
        ],
        ink,
    );
}

fn arrow_head(pixmap: &mut Pixmap, tip: (f32, f32), dir: (f32, f32), size: f32, ink: Color) {
    let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt().max(0.001);
    let dx = dir.0 / len;
    let dy = dir.1 / len;
    fill_polygon(
        pixmap,
        &[
            tip,
            (tip.0 - dx * size + -dy * size * 0.48, tip.1 - dy * size + dx * size * 0.48),
            (tip.0 - dx * size - -dy * size * 0.48, tip.1 - dy * size - dx * size * 0.48),
        ],
        ink,
    );
}

fn ring_arc(
    pixmap: &mut Pixmap,
    center: (f32, f32),
    outer: f32,
    inner: f32,
    start: f32,
    end: f32,
    ink: Color,
) {
    let steps = 32;
    let mut path = PathBuilder::new();
    for index in 0..=steps {
        let angle = start + (end - start) * index as f32 / steps as f32;
        let point = (center.0 + outer * angle.cos(), center.1 + outer * angle.sin());
        if index == 0 {
            path.move_to(point.0, point.1);
        } else {
            path.line_to(point.0, point.1);
        }
    }
    for index in (0..=steps).rev() {
        let angle = start + (end - start) * index as f32 / steps as f32;
        let point = (center.0 + inner * angle.cos(), center.1 + inner * angle.sin());
        path.line_to(point.0, point.1);
    }
    path.close();
    fill_path(pixmap, path.finish().unwrap(), ink, FillRule::Winding);
}

fn wave(pixmap: &mut Pixmap, start: (f32, f32), end: (f32, f32), amplitude: f32, thickness: f32, ink: Color) {
    let steps = 16;
    let mut points = Vec::with_capacity(steps + 1);
    for index in 0..=steps {
        let t = index as f32 / steps as f32;
        points.push((
            start.0 + (end.0 - start.0) * t,
            start.1 + (end.1 - start.1) * t + (t * std::f32::consts::TAU).sin() * amplitude,
        ));
    }
    for pair in points.windows(2) {
        thick_line(pixmap, pair[0], pair[1], thickness, ink);
    }
}

fn fill_even_odd(pixmap: &mut Pixmap, rings: &[[(f32, f32); 4]], ink: Color) {
    let mut path = PathBuilder::new();
    for ring in rings {
        path.move_to(ring[0].0, ring[0].1);
        for point in &ring[1..] {
            path.line_to(point.0, point.1);
        }
        path.close();
    }
    fill_path(pixmap, path.finish().unwrap(), ink, FillRule::EvenOdd);
}

fn fill_polygon(pixmap: &mut Pixmap, points: &[(f32, f32)], ink: Color) {
    let mut path = PathBuilder::new();
    path.move_to(points[0].0, points[0].1);
    for point in &points[1..] {
        path.line_to(point.0, point.1);
    }
    path.close();
    fill_path(pixmap, path.finish().unwrap(), ink, FillRule::Winding);
}

fn fill_path(pixmap: &mut Pixmap, path: tiny_skia::Path, ink: Color, rule: FillRule) {
    let mut paint = Paint::default();
    paint.set_color(ink);
    pixmap.fill_path(&path, &paint, rule, Transform::identity(), None);
}

fn fill_rect(pixmap: &mut Pixmap, rect: Rect, radius: f32, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    let path = rounded(rect, radius);
    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn fill_circle(pixmap: &mut Pixmap, cx: f32, cy: f32, radius: f32, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    let path = PathBuilder::from_circle(cx, cy, radius).unwrap();
    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn rounded(rect: Rect, radius: f32) -> tiny_skia::Path {
    let radius = radius.min(rect.w * 0.5).min(rect.h * 0.5);
    let mut path = PathBuilder::new();
    let x = rect.x;
    let y = rect.y;
    let right = rect.x + rect.w;
    let bottom = rect.y + rect.h;
    path.move_to(x + radius, y);
    path.line_to(right - radius, y);
    path.quad_to(right, y, right, y + radius);
    path.line_to(right, bottom - radius);
    path.quad_to(right, bottom, right - radius, bottom);
    path.line_to(x + radius, bottom);
    path.quad_to(x, bottom, x, bottom - radius);
    path.line_to(x, y + radius);
    path.quad_to(x, y, x + radius, y);
    path.close();
    path.finish().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EcuView;

    fn linked(id: &str) -> EcuView {
        let mut ecu = EcuView::default();
        ecu.linked = true;
        ecu.firmware = id.to_string();
        ecu.calibration = crate::firmware::find(id).unwrap().calibration.to_string();
        ecu.readings.push(("обороты".into(), 860.0));
        ecu
    }

    #[test]
    fn readings_frames_differ_between_the_two_firmwares() {
        let mut fonts = Typeface::load();
        let climate = Climate::default();
        let m74 = render(Screen::Readings, &climate, &linked("итэлма-м74"), &mut fonts);
        let january = render(Screen::Readings, &climate, &linked("январь-7.2"), &mut fonts);
        assert_eq!(m74.width(), 1024);
        assert_eq!(m74.height(), 600);
        assert_ne!(m74.data(), january.data());
        let silent = render(Screen::Readings, &climate, &EcuView::default(), &mut fonts);
        assert_ne!(silent.data(), m74.data());
    }
}

fn color(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}
