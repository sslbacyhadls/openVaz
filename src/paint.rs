use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use fontdue::{Font, FontSettings};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::firmware::{self, Channel};
use crate::layout::{self, DriveHit, Hit, Rect};
use crate::model::{
    celsius_from_unit, Climate, EcuView, Screen, PROFILE_TITLE, TEMP_MAX_C, TEMP_MIN_C,
};
use crate::vehicle::Vehicle;

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
        Screen::Drive => draw_drive(&mut pixmap, fonts, ecu),
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
    text(pixmap, fonts, 48.0, 84.0, 48.0, true, color(0xf4, 0xf1, 0xea), "Климат");
    let profile_w = fonts.width(PROFILE_TITLE, 24.0, false);
    text(
        pixmap,
        fonts,
        layout::LAYOUT_W - 48.0 - profile_w,
        78.0,
        24.0,
        false,
        color(0x9a, 0xa3, 0xad),
        PROFILE_TITLE,
    );

    let track = layout::slider_track();
    let cold = format!("{TEMP_MIN_C}°");
    let hot = format!("{TEMP_MAX_C}°");
    text(pixmap, fonts, 48.0, track.y + 16.0, 24.0, false, color(0x7e, 0xb6, 0xd9), &cold);
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
    hint(pixmap, fonts, "Проведите пальцем влево — ЭБУ");
}

fn draw_ecu(pixmap: &mut Pixmap, fonts: &mut Typeface, ecu: &EcuView) {
    text(pixmap, fonts, 48.0, 84.0, 48.0, true, color(0xf4, 0xf1, 0xea), "ЭБУ");
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
        48.0,
        150.0,
        "Температура охлаждающей жидкости",
        &coolant,
        coolant_color,
    );
    card(pixmap, fonts, 48.0, 372.0, "Коды ошибок", &errors, error_color);
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
                48.0,
                620.0,
                28.0,
                false,
                color(0xf4, 0xf1, 0xea),
                clutch,
            );
        }
    }
    hint(pixmap, fonts, "Влево — показания, вправо — климат");
}

fn draw_readings(pixmap: &mut Pixmap, fonts: &mut Typeface, ecu: &EcuView) {
    text(pixmap, fonts, 48.0, 84.0, 48.0, true, color(0xf4, 0xf1, 0xea), "Показания");
    if !ecu.linked {
        card(pixmap, fonts, 48.0, 150.0, "Связь с блоком", "нет связи с ЭБУ", color(0xd4, 0x55, 0x4a));
        hint(pixmap, fonts, "Влево — движение, вправо — ЭБУ");
        return;
    }
    let firmware = firmware::find(&ecu.calibration);
    let title = if ecu.calibration.is_empty() {
        ecu.family.clone()
    } else {
        format!("{}  {}", ecu.family, ecu.calibration)
    };
    text(pixmap, fonts, 48.0, 132.0, 24.0, false, color(0x9a, 0xa3, 0xad), &title);
    let channels: Vec<&Channel> = firmware
        .map(|item| item.channels().collect())
        .unwrap_or_default();
    let rows = 9;
    for (index, channel) in channels.iter().enumerate() {
        let column = index / rows;
        let row = index % rows;
        let x = 48.0 + column as f32 * 900.0;
        let y = 180.0 + row as f32 * 78.0;
        let value = ecu
            .readings
            .get(channel.key)
            .map(|number| firmware::format_channel(channel, *number))
            .unwrap_or_else(|| "нет данных".to_string());
        let unit = if channel.unit.is_empty() {
            value
        } else {
            format!("{value} {}", channel.unit)
        };
        text(pixmap, fonts, x, y + 28.0, 22.0, false, color(0x9a, 0xa3, 0xad), channel.title);
        text(pixmap, fonts, x + 560.0, y + 28.0, 26.0, true, color(0xf4, 0xf1, 0xea), &unit);
    }
    hint(pixmap, fonts, "Влево — движение, вправо — ЭБУ");
}

fn draw_drive(pixmap: &mut Pixmap, fonts: &mut Typeface, ecu: &EcuView) {
    text(pixmap, fonts, 48.0, 84.0, 48.0, true, color(0xf4, 0xf1, 0xea), "Движение");
    if !ecu.linked {
        card(pixmap, fonts, 48.0, 150.0, "Автомобиль", "нет связи с ЭБУ", color(0xd4, 0x55, 0x4a));
        hint(pixmap, fonts, "Вправо — показания");
        return;
    }
    let speed = ecu.readings.get("скорость").copied().unwrap_or(0.0);
    let signed = if ecu.gear < 0 { -speed } else { speed };
    let rpm = ecu.readings.get("обороты").copied().unwrap_or(0.0);
    text(
        pixmap,
        fonts,
        80.0,
        280.0,
        120.0,
        true,
        color(0xf4, 0xf1, 0xea),
        &format!("{}", signed.round() as i32),
    );
    text(pixmap, fonts, 80.0, 330.0, 28.0, false, color(0x9a, 0xa3, 0xad), "км/ч");
    let center = (430.0, 560.0);
    let start = 3.55_f32;
    let sweep = 2.35_f32;
    ring_arc(pixmap, center, 168.0, 146.0, start, start + sweep, color(0x2a, 0x33, 0x40));
    let fraction = (rpm / 7000.0).clamp(0.0, 1.0);
    if fraction > 0.01 {
        ring_arc(
            pixmap,
            center,
            168.0,
            146.0,
            start,
            start + sweep * fraction,
            color(0xe8, 0xa3, 0x17),
        );
    }
    let rpm_label = format!("{}", rpm.round() as i32);
    let rpm_w = fonts.width(&rpm_label, 36.0, true);
    text(pixmap, fonts, center.0 - rpm_w * 0.5, center.1 + 12.0, 36.0, true, color(0xf4, 0xf1, 0xea), &rpm_label);
    text(pixmap, fonts, center.0 - 70.0, center.1 + 48.0, 20.0, false, color(0x9a, 0xa3, 0xad), "об/мин");
    draw_road(pixmap, speed, ecu.gear < 0);
    for (hit, rect) in layout::drive_pedals() {
        let amount = match hit {
            DriveHit::Gas => ecu.gas,
            DriveHit::Brake => ecu.brake,
            DriveHit::Clutch => ecu.clutch_pedal,
            _ => 0.0,
        };
        let ink = match hit {
            DriveHit::Gas => color(0xe8, 0xa3, 0x17),
            DriveHit::Brake => color(0xd4, 0x55, 0x4a),
            _ => color(0xf4, 0xf1, 0xea),
        };
        fill_rect(pixmap, rect, 18.0, color(0x1c, 0x24, 0x30));
        let filled = Rect { x: rect.x, y: rect.y + rect.h * (1.0 - amount), w: rect.w, h: rect.h * amount };
        if amount > 0.02 {
            fill_rect(pixmap, filled, 0.0, ink);
        }
        let label = match hit {
            DriveHit::Gas => "газ",
            DriveHit::Brake => "тормоз",
            DriveHit::Clutch => "сцепл.",
            _ => "",
        };
        let label_ink = if amount > 0.55 { color(0x12, 0x16, 0x1c) } else { color(0xf4, 0xf1, 0xea) };
        text(pixmap, fonts, rect.x + 18.0, rect.y + rect.h - 28.0, 20.0, false, label_ink, label);
    }
    for (hit, rect) in layout::drive_gears() {
        let code = match hit {
            DriveHit::Gear(code) => code,
            _ => 0,
        };
        let on = ecu.gear == code;
        fill_rect(pixmap, rect, 12.0, if on { color(0xe8, 0xa3, 0x17) } else { color(0x1c, 0x24, 0x30) });
        let name = match code {
            -1 => "R",
            0 => "N",
            gear => return_gear(gear),
        };
        text(
            pixmap,
            fonts,
            rect.x + 36.0,
            rect.y + 46.0,
            28.0,
            true,
            if on { color(0x12, 0x16, 0x1c) } else { color(0xf4, 0xf1, 0xea) },
            name,
        );
    }
    let motor = match ecu.motor {
        1 => "стартер",
        2 => "мотор работает",
        3 => "заглох",
        _ => "зажигание выключено",
    };
    text(pixmap, fonts, 900.0, 820.0, 24.0, false, color(0xf4, 0xf1, 0xea), motor);
    let ignition = layout::Rect { x: 900.0, y: 860.0, w: 160.0, h: 72.0 };
    let starter = layout::Rect { x: 1080.0, y: 860.0, w: 160.0, h: 72.0 };
    fill_rect(pixmap, ignition, 12.0, color(0x1c, 0x24, 0x30));
    fill_rect(pixmap, starter, 12.0, color(0x1c, 0x24, 0x30));
    text(pixmap, fonts, 924.0, 906.0, 22.0, false, color(0xf4, 0xf1, 0xea), "зажигание");
    text(pixmap, fonts, 1112.0, 906.0, 22.0, false, color(0xf4, 0xf1, 0xea), "стартер");
    hint(pixmap, fonts, "Удерживайте педаль. Вправо — показания");
}

fn return_gear(gear: i32) -> &'static str {
    match gear {
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        _ => "N",
    }
}

fn draw_road(pixmap: &mut Pixmap, speed: f32, reverse: bool) {
    let road = Rect { x: 80.0, y: 700.0, w: 1100.0, h: 70.0 };
    fill_rect(pixmap, road, 8.0, color(0x1c, 0x24, 0x30));
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs_f32())
        .unwrap_or(0.0);
    let direction = if reverse { -1.0 } else { 1.0 };
    let shift = (seconds * speed.max(0.0) * 28.0 * direction).rem_euclid(90.0);
    let mut x = road.x + shift;
    while x < road.x + road.w {
        if x + 36.0 > road.x {
            fill_rect(
                pixmap,
                Rect { x: x.max(road.x), y: road.y + 30.0, w: 36.0, h: 8.0 },
                0.0,
                color(0xe8, 0xa3, 0x17),
            );
        }
        x += 90.0;
    }
}

fn card(pixmap: &mut Pixmap, fonts: &mut Typeface, x: f32, y: f32, title: &str, value: &str, value_color: Color) {
    let rect = Rect {
        x,
        y,
        w: 1040.0,
        h: 190.0,
    };
    fill_rect(pixmap, rect, 18.0, color(0x1c, 0x24, 0x30));
    text(pixmap, fonts, x + 32.0, y + 64.0, 26.0, false, color(0x9a, 0xa3, 0xad), title);
    text(pixmap, fonts, x + 32.0, y + 136.0, 42.0, true, value_color, value);
}

fn button(pixmap: &mut Pixmap, rect: Rect, on: bool, active: Color, hit: Hit) {
    let fill = if on { active } else { color(0x1c, 0x24, 0x30) };
    let ink = if on { color(0x12, 0x16, 0x1c) } else { color(0xf4, 0xf1, 0xea) };
    fill_rect(pixmap, rect, 22.0, fill);
    let size = 78.0;
    let origin = (rect.x + (rect.w - size) * 0.5, rect.y + (rect.h - size) * 0.5);
    match hit {
        Hit::Face => icon_face(pixmap, origin, size, ink),
        Hit::Feet => icon_feet(pixmap, origin, size, ink),
        Hit::Glass => icon_glass(pixmap, origin, size, ink),
        Hit::Recirc => icon_recirc(pixmap, origin, size, ink),
        Hit::Ac => icon_snowflake(pixmap, origin, size, ink),
        Hit::Slider => {}
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

fn hint(pixmap: &mut Pixmap, fonts: &mut Typeface, label: &str) {
    let size = 22.0;
    let width = fonts.width(label, size, false);
    text(
        pixmap,
        fonts,
        (layout::LAYOUT_W - width) * 0.5,
        layout::LAYOUT_H - 78.0,
        size,
        false,
        color(0x9a, 0xa3, 0xad),
        label,
    );
}

fn draw_dots(pixmap: &mut Pixmap, screen: Screen) {
    let y = layout::LAYOUT_H - 36.0;
    let active = color(0xf4, 0xf1, 0xea);
    let idle = color(0x3c, 0x46, 0x54);
    let order = [Screen::Climate, Screen::Ecu, Screen::Readings, Screen::Drive];
    for (index, item) in order.iter().enumerate() {
        fill_circle(
            pixmap,
            layout::LAYOUT_W * 0.5 + (index as f32 - 1.5) * 28.0,
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

fn color(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

pub const DASH_W: f32 = 1200.0;
pub const DASH_H: f32 = 760.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashHit {
    Gas,
    Brake,
    Clutch,
    Gear(i32),
    Ignition,
    Starter,
}

pub fn render_dashboard(vehicle: &Vehicle, linked: bool, fonts: &mut Typeface) -> Pixmap {
    let mut pixmap = Pixmap::new(DASH_W as u32, DASH_H as u32).expect("pixmap");
    pixmap.fill(color(0x12, 0x16, 0x1c));
    let firmware = vehicle.firmware();
    text(
        &mut pixmap,
        fonts,
        40.0,
        58.0,
        32.0,
        true,
        color(0xf4, 0xf1, 0xea),
        &format!("{}  {}", firmware.family, firmware.id),
    );
    text(
        &mut pixmap,
        fonts,
        40.0,
        96.0,
        20.0,
        false,
        color(0x9a, 0xa3, 0xad),
        &format!("мотор {}   {}", firmware.engine, vehicle.motor().label()),
    );
    let link = if linked { "панель на связи" } else { "жду панель" };
    let link_color = if linked { color(0x7d, 0xc6, 0x7a) } else { color(0xd4, 0x55, 0x4a) };
    text(&mut pixmap, fonts, 900.0, 58.0, 22.0, false, link_color, link);

    let speed = vehicle.speed_kmh().round() as i32;
    text(
        &mut pixmap,
        fonts,
        48.0,
        250.0,
        108.0,
        true,
        color(0xf4, 0xf1, 0xea),
        &format!("{speed}"),
    );
    text(&mut pixmap, fonts, 52.0, 292.0, 24.0, false, color(0x9a, 0xa3, 0xad), "км/ч");
    let gear = vehicle.driver().gear.label();
    text(&mut pixmap, fonts, 280.0, 250.0, 64.0, true, color(0xe8, 0xa3, 0x17), gear);

    let center = (230.0, 430.0);
    let start = 3.55_f32;
    let sweep = 2.35_f32;
    ring_arc(&mut pixmap, center, 132.0, 112.0, start, start + sweep, color(0x2a, 0x33, 0x40));
    let fraction = (vehicle.rpm() / firmware.rev_limit).clamp(0.0, 1.0);
    if fraction > 0.01 {
        ring_arc(
            &mut pixmap,
            center,
            132.0,
            112.0,
            start,
            start + sweep * fraction,
            color(0xe8, 0xa3, 0x17),
        );
    }
    let angle = start + sweep * fraction;
    thick_line(
        &mut pixmap,
        center,
        (center.0 + angle.cos() * 96.0, center.1 + angle.sin() * 96.0),
        4.0,
        color(0xf4, 0xf1, 0xea),
    );
    fill_circle(&mut pixmap, center.0, center.1, 8.0, color(0xf4, 0xf1, 0xea));
    let rpm = format!("{}", vehicle.rpm().round() as i32);
    let rpm_w = fonts.width(&rpm, 28.0, true);
    text(
        &mut pixmap,
        fonts,
        center.0 - rpm_w * 0.5,
        center.1 + 46.0,
        28.0,
        true,
        color(0xf4, 0xf1, 0xea),
        &rpm,
    );
    text(&mut pixmap, fonts, center.0 - 36.0, center.1 + 74.0, 16.0, false, color(0x9a, 0xa3, 0xad), "об/мин");
    dash_road(&mut pixmap, vehicle.channel("скорость"), vehicle.speed_kmh() < 0.0);

    let mut rows = vec![
        ("Жидкость".to_string(), format!("{} °C", vehicle.coolant().round() as i32)),
        ("Дроссель".to_string(), format!("{:.0} %", vehicle.channel("дроссель"))),
        ("Впрыск".to_string(), format!("{:.2} мс", vehicle.channel("впрыск"))),
        ("Расход воздуха".to_string(), format!("{:.1} кг/ч", vehicle.channel("дмрв"))),
        ("Кислород".to_string(), format!("{:.2} В", vehicle.channel("кислород"))),
    ];
    if firmware.has_channel("педаль") {
        rows.insert(1, ("Педаль".to_string(), format!("{:.0} %", vehicle.channel("педаль"))));
    }
    if firmware.has_channel("кислород2") {
        rows.push(("Кислород 2".to_string(), format!("{:.2} В", vehicle.channel("кислород2"))));
    }
    rows.push(("Коррекция".to_string(), format!("{:.2}", vehicle.channel("коррекция"))));
    rows.push(("Напряжение".to_string(), format!("{:.1} В", vehicle.channel("напряжение"))));
    rows.push(("Ошибки".to_string(), vehicle.error().unwrap_or("нет").to_string()));
    for (index, (title, value)) in rows.iter().enumerate() {
        let y = 150.0 + index as f32 * 50.0;
        text(&mut pixmap, fonts, 520.0, y, 18.0, false, color(0x9a, 0xa3, 0xad), title);
        let ink = if *title == "Ошибки" && value != "нет" {
            color(0xd4, 0x55, 0x4a)
        } else {
            color(0xf4, 0xf1, 0xea)
        };
        text(&mut pixmap, fonts, 520.0, y + 28.0, 24.0, true, ink, value);
    }

    let driver = vehicle.driver();
    for (hit, rect) in dash_pedals() {
        let amount = match hit {
            DashHit::Gas => driver.gas,
            DashHit::Brake => driver.brake,
            DashHit::Clutch => driver.clutch,
            _ => 0.0,
        };
        let ink = match hit {
            DashHit::Gas => color(0xe8, 0xa3, 0x17),
            DashHit::Brake => color(0xd4, 0x55, 0x4a),
            _ => color(0xc5, 0xcc, 0xd4),
        };
        fill_rect(&mut pixmap, rect, 16.0, color(0x1c, 0x24, 0x30));
        if amount > 0.02 {
            fill_rect(
                &mut pixmap,
                Rect { x: rect.x, y: rect.y + rect.h * (1.0 - amount), w: rect.w, h: rect.h * amount },
                0.0,
                ink,
            );
        }
        let label = match hit {
            DashHit::Gas => "газ",
            DashHit::Brake => "тормоз",
            DashHit::Clutch => "сцепл.",
            _ => "",
        };
        let label_ink = if amount > 0.55 { color(0x12, 0x16, 0x1c) } else { color(0xf4, 0xf1, 0xea) };
        text(&mut pixmap, fonts, rect.x + 10.0, rect.y + rect.h - 22.0, 16.0, false, label_ink, label);
    }

    for (hit, rect) in dash_gears() {
        let code = match hit {
            DashHit::Gear(code) => code,
            _ => 0,
        };
        let on = driver.gear.code() == code;
        fill_rect(
            &mut pixmap,
            rect,
            10.0,
            if on { color(0xe8, 0xa3, 0x17) } else { color(0x1c, 0x24, 0x30) },
        );
        let name = match code {
            -1 => "R",
            0 => "N",
            1 => "1",
            2 => "2",
            3 => "3",
            4 => "4",
            _ => "5",
        };
        text(
            &mut pixmap,
            fonts,
            rect.x + 26.0,
            rect.y + 40.0,
            24.0,
            true,
            if on { color(0x12, 0x16, 0x1c) } else { color(0xf4, 0xf1, 0xea) },
            name,
        );
    }
    let ignition_on = !matches!(vehicle.motor(), crate::engine::Motor::Off);
    button_caption(
        &mut pixmap,
        fonts,
        Rect { x: 760.0, y: 668.0, w: 180.0, h: 64.0 },
        "зажигание",
        ignition_on,
    );
    button_caption(
        &mut pixmap,
        fonts,
        Rect { x: 960.0, y: 668.0, w: 180.0, h: 64.0 },
        "стартер",
        driver.crank,
    );
    pixmap
}

pub fn hit_dashboard(x: f32, y: f32) -> Option<DashHit> {
    for (hit, rect) in dash_pedals() {
        if rect.contains(x, y) {
            return Some(hit);
        }
    }
    for (hit, rect) in dash_gears() {
        if rect.contains(x, y) {
            return Some(hit);
        }
    }
    if (Rect { x: 760.0, y: 668.0, w: 180.0, h: 64.0 }).contains(x, y) {
        return Some(DashHit::Ignition);
    }
    if (Rect { x: 960.0, y: 668.0, w: 180.0, h: 64.0 }).contains(x, y) {
        return Some(DashHit::Starter);
    }
    None
}

fn dash_pedals() -> [(DashHit, Rect); 3] {
    let y = 140.0;
    let h = 500.0;
    let w = 84.0;
    [
        (DashHit::Clutch, Rect { x: 900.0, y, w, h }),
        (DashHit::Brake, Rect { x: 1000.0, y, w, h }),
        (DashHit::Gas, Rect { x: 1100.0, y, w, h }),
    ]
}

fn dash_gears() -> [(DashHit, Rect); 7] {
    let keys = [
        DashHit::Gear(1),
        DashHit::Gear(2),
        DashHit::Gear(3),
        DashHit::Gear(4),
        DashHit::Gear(5),
        DashHit::Gear(0),
        DashHit::Gear(-1),
    ];
    let mut row = [(DashHit::Gear(0), Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }); 7];
    for (index, hit) in keys.into_iter().enumerate() {
        row[index] = (
            hit,
            Rect { x: 40.0 + index as f32 * 84.0, y: 668.0, w: 72.0, h: 64.0 },
        );
    }
    row
}

fn button_caption(pixmap: &mut Pixmap, fonts: &mut Typeface, rect: Rect, label: &str, on: bool) {
    fill_rect(
        pixmap,
        rect,
        12.0,
        if on { color(0xe8, 0xa3, 0x17) } else { color(0x1c, 0x24, 0x30) },
    );
    text(
        pixmap,
        fonts,
        rect.x + 18.0,
        rect.y + 40.0,
        22.0,
        false,
        if on { color(0x12, 0x16, 0x1c) } else { color(0xf4, 0xf1, 0xea) },
        label,
    );
}

fn dash_road(pixmap: &mut Pixmap, speed: f32, reverse: bool) {
    let road = Rect { x: 40.0, y: 575.0, w: 460.0, h: 48.0 };
    fill_rect(pixmap, road, 8.0, color(0x1c, 0x24, 0x30));
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs_f32())
        .unwrap_or(0.0);
    let direction = if reverse { -1.0 } else { 1.0 };
    let shift = (seconds * speed.max(0.0) * 24.0 * direction).rem_euclid(70.0);
    let mut x = road.x - 70.0 + shift;
    while x < road.x + road.w {
        let mark = Rect { x, y: road.y + 20.0, w: 28.0, h: 6.0 };
        if mark.x + mark.w > road.x && mark.x < road.x + road.w {
            fill_rect(pixmap, mark, 0.0, color(0xe8, 0xa3, 0x17));
        }
        x += 70.0;
    }
}
