use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use fontdue::{Font, FontSettings};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::engine::Gear;
use crate::layout::{self, Hit, Rect};
use crate::model::{
    celsius_from_unit, Climate, EcuView, Screen, MIN_FAN, PROFILE_TITLE, TEMP_MAX_C, TEMP_MIN_C,
};
use crate::vehicle::Vehicle;

const HEADER_SIZE: f32 = 32.0;
const VALUE_SIZE: f32 = 72.0;
const LABEL_SIZE: f32 = 24.0;
const HINT_SIZE: f32 = 20.0;
const MARGIN: f32 = 24.0;

const BACKGROUND: u32 = 0x12161c;
const PANEL: u32 = 0x1c2430;
const INK: u32 = 0xf4f1ea;
const MUTED: u32 = 0x9aa3ad;
const TRACK: u32 = 0x3c4654;
const AMBER: u32 = 0xe8a317;
const AC_BLUE: u32 = 0x3d9ee0;
const RED: u32 = 0xd4554a;

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
    pixmap.fill(color(BACKGROUND));
    match screen {
        Screen::Climate => draw_climate(&mut pixmap, fonts, climate, ecu),
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

fn header(pixmap: &mut Pixmap, fonts: &mut Typeface, title: &str) {
    text(
        pixmap,
        fonts,
        (MARGIN, 64.0),
        HEADER_SIZE,
        true,
        color(INK),
        title,
    );
    let width = fonts.width(PROFILE_TITLE, LABEL_SIZE, false);
    text(
        pixmap,
        fonts,
        (layout::LAYOUT_W - MARGIN - width, 58.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        PROFILE_TITLE,
    );
}

fn draw_climate(pixmap: &mut Pixmap, fonts: &mut Typeface, climate: &Climate, ecu: &EcuView) {
    header(pixmap, fonts, "Климат");
    let mode = layout::auto_button();
    fill_rect(
        pixmap,
        mode,
        16.0,
        color(if climate.automatic { AMBER } else { PANEL }),
    );
    let mode_label = if climate.automatic {
        "Климат"
    } else {
        "Вручную"
    };
    let mode_ink = color(if climate.automatic { BACKGROUND } else { INK });
    let mode_w = fonts.width(mode_label, LABEL_SIZE, true);
    text(
        pixmap,
        fonts,
        (mode.x + (mode.w - mode_w) * 0.5, mode.y + 40.0),
        LABEL_SIZE,
        true,
        mode_ink,
        mode_label,
    );
    draw_cabin(
        pixmap,
        fonts,
        climate,
        ecu.linked.then_some(ecu.cabin_c).flatten(),
    );
    text(
        pixmap,
        fonts,
        (376.0, 112.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        "Задание температуры",
    );
    let degrees = format!("{} °C", celsius_from_unit(climate.temperature));
    let width = fonts.width(&degrees, VALUE_SIZE, true);
    text(
        pixmap,
        fonts,
        (688.0 - width * 0.5, 184.0),
        VALUE_SIZE,
        true,
        color(AMBER),
        &degrees,
    );

    let track = layout::slider_track();
    text(
        pixmap,
        fonts,
        (376.0, track.y + 16.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        &format!("{TEMP_MIN_C}°"),
    );
    text(
        pixmap,
        fonts,
        (952.0, track.y + 16.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        &format!("{TEMP_MAX_C}°"),
    );
    fill_rect(pixmap, track, 7.0, color(TRACK));
    let knob_x = track.x + track.w * climate.temperature;
    if knob_x > track.x {
        fill_rect(
            pixmap,
            Rect {
                w: knob_x - track.x,
                ..track
            },
            7.0,
            color(AMBER),
        );
    }
    fill_circle(pixmap, knob_x, track.y + track.h * 0.5, 20.0, color(INK));

    for (hit, rect) in layout::mode_buttons()
        .into_iter()
        .chain(layout::lower_buttons())
    {
        let on = match hit {
            Hit::Face => climate.face,
            Hit::Feet => climate.feet,
            Hit::Glass => climate.glass,
            Hit::Recirc => climate.recirculation,
            Hit::Ac => climate.air_conditioner,
            _ => false,
        };
        button(pixmap, fonts, rect, on, hit);
    }
    text(
        pixmap,
        fonts,
        (376.0, 288.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        "Вентилятор",
    );
    for (hit, rect) in layout::fan_buttons() {
        let Hit::Fan(speed) = hit else { continue };
        let on = (climate.fan - speed as f32 * MIN_FAN).abs() < 0.001;
        let fill = if on { AMBER } else { PANEL };
        let ink = if on { BACKGROUND } else { INK };
        fill_rect(pixmap, rect, 16.0, color(fill));
        let label = if speed == 0 {
            "Выкл".to_string()
        } else {
            speed.to_string()
        };
        let width = fonts.width(&label, LABEL_SIZE, true);
        text(
            pixmap,
            fonts,
            (rect.x + (rect.w - width) * 0.5, rect.y + 40.0),
            LABEL_SIZE,
            true,
            color(ink),
            &label,
        );
    }
    hint(pixmap, fonts, "Проведите пальцем влево — показания ЭБУ");
}

fn draw_cabin(pixmap: &mut Pixmap, fonts: &mut Typeface, climate: &Climate, cabin_c: Option<f32>) {
    fill_rect(
        pixmap,
        Rect {
            x: MARGIN,
            y: 96.0,
            w: 304.0,
            h: 280.0,
        },
        18.0,
        color(PANEL),
    );
    text(
        pixmap,
        fonts,
        (48.0, 136.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        "Потоки воздуха",
    );
    if let Some(cabin) = cabin_c {
        let label = format!("салон {cabin:.0}°");
        text(
            pixmap,
            fonts,
            (48.0, 360.0),
            LABEL_SIZE,
            false,
            color(INK),
            &label,
        );
    }
    let glass = color(if climate.glass { AMBER } else { TRACK });
    thick_line(pixmap, (56.0, 264.0), (96.0, 168.0), 6.0, glass);
    thick_line(pixmap, (96.0, 168.0), (176.0, 168.0), 4.0, color(TRACK));
    thick_line(pixmap, (56.0, 328.0), (296.0, 328.0), 4.0, color(TRACK));
    // Side view: seat, head, torso and bent legs.
    thick_line(pixmap, (280.0, 224.0), (272.0, 288.0), 8.0, color(TRACK));
    thick_line(pixmap, (272.0, 288.0), (216.0, 288.0), 8.0, color(TRACK));
    fill_circle(pixmap, 244.0, 192.0, 16.0, color(INK));
    for (start, end) in [
        ((244.0, 216.0), (240.0, 264.0)),
        ((240.0, 264.0), (208.0, 272.0)),
        ((208.0, 272.0), (192.0, 312.0)),
    ] {
        thick_line(pixmap, start, end, 10.0, color(INK));
    }
    for (on, start, end) in [
        (climate.glass, (144.0, 248.0), (112.0, 192.0)),
        (climate.face, (152.0, 224.0), (212.0, 224.0)),
        (climate.feet, (144.0, 280.0), (176.0, 304.0)),
    ] {
        let ink = color(if on { AMBER } else { TRACK });
        thick_line(pixmap, start, end, 5.0, ink);
        arrow_head(pixmap, end, (end.0 - start.0, end.1 - start.1), 12.0, ink);
    }
    if climate.recirculation {
        icon_recirc(pixmap, (64.0, 276.0), 40.0, color(AMBER));
    } else {
        thick_line(pixmap, (32.0, 288.0), (88.0, 288.0), 4.0, color(MUTED));
        arrow_head(pixmap, (88.0, 288.0), (1.0, 0.0), 10.0, color(MUTED));
    }
    if climate.air_conditioner {
        icon_snowflake(pixmap, (272.0, 160.0), 32.0, color(AC_BLUE));
    }
}

fn draw_ecu(pixmap: &mut Pixmap, fonts: &mut Typeface, ecu: &EcuView) {
    header(pixmap, fonts, "ЭБУ");
    if !ecu.linked {
        let label = "нет связи с ЭБУ";
        let width = fonts.width(label, HEADER_SIZE, true);
        text(
            pixmap,
            fonts,
            ((layout::LAYOUT_W - width) * 0.5, 304.0),
            HEADER_SIZE,
            true,
            color(RED),
            label,
        );
    } else {
        let (coolant, ink) = match ecu.coolant_c {
            Some(degrees) => (
                format!("{degrees} °C"),
                if degrees >= 105 { RED } else { INK },
            ),
            None => ("нет данных".to_string(), MUTED),
        };
        card(
            pixmap,
            fonts,
            112.0,
            "Температура охлаждающей жидкости",
            &coolant,
            color(ink),
        );
        let errors = if ecu.errors.is_empty() {
            "нет".to_string()
        } else {
            ecu.errors.join(" ")
        };
        card(
            pixmap,
            fonts,
            284.0,
            "Коды ошибок",
            &errors,
            color(if ecu.errors.is_empty() { MUTED } else { RED }),
        );
        let clutch = match ecu.clutch {
            Some(true) => "Муфта кондиционера включена",
            Some(false) => "Муфта кондиционера выключена",
            None => "Нет данных о муфте кондиционера",
        };
        text(
            pixmap,
            fonts,
            (MARGIN, 488.0),
            LABEL_SIZE,
            false,
            color(INK),
            clutch,
        );
    }
    hint(pixmap, fonts, "Проведите пальцем влево — показания");
}

fn card(pixmap: &mut Pixmap, fonts: &mut Typeface, y: f32, title: &str, value: &str, ink: Color) {
    let rect = Rect {
        x: MARGIN,
        y,
        w: layout::LAYOUT_W - MARGIN * 2.0,
        h: 144.0,
    };
    fill_rect(pixmap, rect, 18.0, color(PANEL));
    text(
        pixmap,
        fonts,
        (MARGIN * 2.0, y + 40.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        title,
    );
    let size = if fonts.width(value, VALUE_SIZE, true) <= rect.w - MARGIN * 2.0 {
        VALUE_SIZE
    } else {
        LABEL_SIZE
    };
    text(
        pixmap,
        fonts,
        (MARGIN * 2.0, y + 120.0),
        size,
        true,
        ink,
        value,
    );
}

fn button(pixmap: &mut Pixmap, fonts: &mut Typeface, rect: Rect, on: bool, hit: Hit) {
    let active = if hit == Hit::Ac { AC_BLUE } else { AMBER };
    let ink = color(if on { BACKGROUND } else { INK });
    fill_rect(pixmap, rect, 18.0, color(if on { active } else { PANEL }));
    let size = 44.0;
    let origin = (rect.x + (rect.w - size) * 0.5, rect.y + 8.0);
    let label = match hit {
        Hit::Face => {
            icon_face(pixmap, origin, size, ink);
            "Лицо"
        }
        Hit::Feet => {
            icon_feet(pixmap, origin, size, ink);
            "Ноги"
        }
        Hit::Glass => {
            icon_glass(pixmap, origin, size, ink);
            "Стекло"
        }
        Hit::Recirc => {
            icon_recirc(pixmap, origin, size, ink);
            "Рецирк."
        }
        Hit::Ac => {
            icon_snowflake(pixmap, origin, size, ink);
            "Кондиционер"
        }
        Hit::Slider | Hit::Fan(_) | Hit::Auto => return,
    };
    let width = fonts.width(label, LABEL_SIZE, false);
    text(
        pixmap,
        fonts,
        (rect.x + (rect.w - width) * 0.5, rect.y + 80.0),
        LABEL_SIZE,
        false,
        ink,
        label,
    );
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
    let inner = [
        px(24.0, 16.0),
        px(54.0, 16.0),
        px(62.0, 60.0),
        px(16.0, 60.0),
    ];
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
                (
                    base.0 + branch.cos() * 10.0 * s,
                    base.1 + branch.sin() * 10.0 * s,
                ),
                3.2 * s,
                ink,
            );
        }
    }
}

fn draw_readings(pixmap: &mut Pixmap, fonts: &mut Typeface, ecu: &EcuView) {
    header(pixmap, fonts, "Показания");
    if !ecu.linked {
        let label = "нет связи с ЭБУ";
        let width = fonts.width(label, HEADER_SIZE, true);
        text(
            pixmap,
            fonts,
            ((layout::LAYOUT_W - width) * 0.5, 300.0),
            HEADER_SIZE,
            true,
            color(RED),
            label,
        );
        hint(pixmap, fonts, "Проведите пальцем вправо — ЭБУ");
        return;
    }
    if let Some(heading) = ecu.readings_heading() {
        let width = fonts.width(&heading, LABEL_SIZE, false);
        text(
            pixmap,
            fonts,
            (layout::LAYOUT_W - MARGIN - width, 108.0),
            LABEL_SIZE,
            false,
            color(MUTED),
            &heading,
        );
    }
    let Some(rows) = ecu.reading_rows() else {
        text(
            pixmap,
            fonts,
            (MARGIN, 160.0),
            LABEL_SIZE,
            false,
            color(MUTED),
            "нет данных",
        );
        hint(pixmap, fonts, "Проведите пальцем влево — движение");
        return;
    };
    let per_column = rows.len().div_ceil(2);
    let row_h = 42.0;
    let top = 136.0;
    let col_w = 488.0;
    for (index, (title, value)) in rows.iter().enumerate() {
        let column = index / per_column;
        let row = index % per_column;
        let x = MARGIN + column as f32 * (col_w + 16.0);
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
            color(TRACK),
        );
        let value_w = fonts.width(value, LABEL_SIZE, true);
        let title_room = (col_w - value_w - 16.0).max(40.0);
        let title = fit_label(fonts, title, 20.0, title_room);
        text(
            pixmap,
            fonts,
            (x, y + 36.0),
            20.0,
            false,
            color(MUTED),
            &title,
        );
        text(
            pixmap,
            fonts,
            (x + col_w - value_w, y + 36.0),
            LABEL_SIZE,
            true,
            color(if value == "нет данных" {
                MUTED
            } else {
                INK
            }),
            value,
        );
    }
    hint(pixmap, fonts, "Проведите пальцем влево — движение");
}

fn draw_drive(pixmap: &mut Pixmap, fonts: &mut Typeface, ecu: &EcuView) {
    header(pixmap, fonts, "Движение");
    if !ecu.linked {
        let label = "нет связи с ЭБУ";
        let width = fonts.width(label, HEADER_SIZE, true);
        text(
            pixmap,
            fonts,
            ((layout::LAYOUT_W - width) * 0.5, 300.0),
            HEADER_SIZE,
            true,
            color(RED),
            label,
        );
        hint(pixmap, fonts, "Проведите пальцем вправо — показания");
        return;
    }
    let speed = ecu
        .reading_of("скорость")
        .map(|value| format!("{:.0}", value.round()))
        .unwrap_or_else(|| "нет данных".into());
    let rpm = ecu
        .reading_of("обороты")
        .map(|value| format!("{:.0}", value.round()))
        .unwrap_or_else(|| "нет данных".into());
    text(
        pixmap,
        fonts,
        (MARGIN, 180.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        "Скорость",
    );
    text(
        pixmap,
        fonts,
        (MARGIN, 260.0),
        VALUE_SIZE,
        true,
        color(INK),
        &speed,
    );
    text(
        pixmap,
        fonts,
        (MARGIN, 300.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        "км/ч",
    );
    text(
        pixmap,
        fonts,
        (520.0, 180.0),
        LABEL_SIZE,
        false,
        color(MUTED),
        "Обороты",
    );
    text(
        pixmap,
        fonts,
        (520.0, 260.0),
        VALUE_SIZE,
        true,
        color(INK),
        &rpm,
    );
    let gear = if ecu.gear.is_empty() {
        "нет данных"
    } else {
        ecu.gear.as_str()
    };
    let motor = if ecu.motor.is_empty() {
        "нет данных"
    } else {
        ecu.motor.as_str()
    };
    text(
        pixmap,
        fonts,
        (MARGIN, 380.0),
        LABEL_SIZE,
        false,
        color(INK),
        &format!("передача {gear}    мотор {motor}"),
    );
    let (gas, brake, clutch) = ecu.pedals.unwrap_or((0.0, 0.0, 0.0));
    for (index, (name, value, ink)) in [
        ("газ", gas, AMBER),
        ("тормоз", brake, RED),
        ("сцепление", clutch, INK),
    ]
    .into_iter()
    .enumerate()
    {
        let x = MARGIN + index as f32 * 330.0;
        text(
            pixmap,
            fonts,
            (x, 450.0),
            HINT_SIZE,
            false,
            color(MUTED),
            name,
        );
        fill_rect(
            pixmap,
            Rect {
                x,
                y: 470.0,
                w: 240.0,
                h: 18.0,
            },
            8.0,
            color(PANEL),
        );
        fill_rect(
            pixmap,
            Rect {
                x,
                y: 470.0,
                w: 240.0 * value.clamp(0.0, 1.0),
                h: 18.0,
            },
            8.0,
            color(ink),
        );
    }
    hint(pixmap, fonts, "Проведите пальцем влево — климат");
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
    let size = HINT_SIZE;
    let width = fonts.width(label, size, false);
    text(
        pixmap,
        fonts,
        ((layout::LAYOUT_W - width) * 0.5, layout::LAYOUT_H - 52.0),
        size,
        false,
        color(MUTED),
        label,
    );
}

fn draw_dots(pixmap: &mut Pixmap, screen: Screen) {
    let screens = [
        Screen::Climate,
        Screen::Ecu,
        Screen::Readings,
        Screen::Drive,
    ];
    let start = (layout::LAYOUT_W - (screens.len() - 1) as f32 * 28.0) * 0.5;
    for (index, candidate) in screens.into_iter().enumerate() {
        fill_circle(
            pixmap,
            start + index as f32 * 28.0,
            layout::LAYOUT_H - 20.0,
            6.0,
            color(if screen == candidate { INK } else { TRACK }),
        );
    }
}

fn text(
    pixmap: &mut Pixmap,
    fonts: &mut Typeface,
    (x, baseline): (f32, f32),
    size: f32,
    bold: bool,
    color: Color,
    label: &str,
) {
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
            (
                tip.0 - dx * size + -dy * size * 0.48,
                tip.1 - dy * size + dx * size * 0.48,
            ),
            (
                tip.0 - dx * size - -dy * size * 0.48,
                tip.1 - dy * size - dx * size * 0.48,
            ),
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
        let point = (
            center.0 + outer * angle.cos(),
            center.1 + outer * angle.sin(),
        );
        if index == 0 {
            path.move_to(point.0, point.1);
        } else {
            path.line_to(point.0, point.1);
        }
    }
    for index in (0..=steps).rev() {
        let angle = start + (end - start) * index as f32 / steps as f32;
        let point = (
            center.0 + inner * angle.cos(),
            center.1 + inner * angle.sin(),
        );
        path.line_to(point.0, point.1);
    }
    path.close();
    fill_path(pixmap, path.finish().unwrap(), ink, FillRule::Winding);
}

fn wave(
    pixmap: &mut Pixmap,
    start: (f32, f32),
    end: (f32, f32),
    amplitude: f32,
    thickness: f32,
    ink: Color,
) {
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
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn fill_circle(pixmap: &mut Pixmap, cx: f32, cy: f32, radius: f32, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    let path = PathBuilder::from_circle(cx, cy, radius).unwrap();
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
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
    pixmap.fill(color(BACKGROUND));
    let firmware = vehicle.firmware();
    let engine = vehicle.engine();
    text(
        &mut pixmap,
        fonts,
        (40.0, 58.0),
        32.0,
        true,
        color(INK),
        &format!("{}  {}", firmware.family, firmware.calibration),
    );
    text(
        &mut pixmap,
        fonts,
        (40.0, 96.0),
        20.0,
        false,
        color(MUTED),
        &format!("мотор {}", engine.motor().label()),
    );
    let (link, link_color) = if linked {
        ("панель на связи", color(0x7dc67a))
    } else {
        ("жду панель", color(RED))
    };
    text(
        &mut pixmap,
        fonts,
        (900.0, 58.0),
        22.0,
        false,
        link_color,
        link,
    );

    let speed = engine.speed_kmh().round() as i32;
    text(
        &mut pixmap,
        fonts,
        (48.0, 250.0),
        108.0,
        true,
        color(INK),
        &format!("{speed}"),
    );
    text(
        &mut pixmap,
        fonts,
        (52.0, 292.0),
        24.0,
        false,
        color(MUTED),
        "км/ч",
    );
    text(
        &mut pixmap,
        fonts,
        (280.0, 250.0),
        64.0,
        true,
        color(AMBER),
        engine.driver().gear.label(),
    );

    let center = (230.0, 430.0);
    let start = 3.55_f32;
    let sweep = 2.35_f32;
    ring_arc(
        &mut pixmap,
        center,
        132.0,
        112.0,
        start,
        start + sweep,
        color(0x2a3340),
    );
    let fraction = (engine.rpm() / firmware.rev_limit).clamp(0.0, 1.0);
    if fraction > 0.01 {
        ring_arc(
            &mut pixmap,
            center,
            132.0,
            112.0,
            start,
            start + sweep * fraction,
            color(AMBER),
        );
    }
    let angle = start + sweep * fraction;
    thick_line(
        &mut pixmap,
        center,
        (center.0 + angle.cos() * 96.0, center.1 + angle.sin() * 96.0),
        4.0,
        color(INK),
    );
    fill_circle(&mut pixmap, center.0, center.1, 8.0, color(INK));
    let rpm = format!("{}", engine.rpm().round() as i32);
    let rpm_w = fonts.width(&rpm, 28.0, true);
    text(
        &mut pixmap,
        fonts,
        (center.0 - rpm_w * 0.5, center.1 + 46.0),
        28.0,
        true,
        color(INK),
        &rpm,
    );
    text(
        &mut pixmap,
        fonts,
        (center.0 - 36.0, center.1 + 74.0),
        16.0,
        false,
        color(MUTED),
        "об/мин",
    );
    dash_road(
        &mut pixmap,
        engine.speed_kmh(),
        engine.driver().gear == Gear::Reverse,
    );

    let mut rows = vec![
        (
            "Жидкость".to_string(),
            format!("{} °C", vehicle.coolant.round() as i32),
        ),
        (
            "Дроссель".to_string(),
            format!("{:.0} %", vehicle.reading("дроссель")),
        ),
        (
            "Впрыск".to_string(),
            format!("{:.2} мс", vehicle.reading("впрыск")),
        ),
        (
            "Расход воздуха".to_string(),
            format!("{:.1} кг/ч", vehicle.reading("дмрв")),
        ),
        (
            "Кислород".to_string(),
            format!("{:.2} В", vehicle.reading("кислород")),
        ),
    ];
    if firmware.has_channel("кислород2") {
        rows.push((
            "Кислород 2".to_string(),
            format!("{:.2} В", vehicle.reading("кислород2")),
        ));
    }
    rows.push((
        "Коррекция".to_string(),
        format!("{:.2}", vehicle.reading("коррекция")),
    ));
    rows.push((
        "Напряжение".to_string(),
        format!("{:.1} В", vehicle.reading("напряжение")),
    ));
    rows.push((
        "Ошибки".to_string(),
        vehicle.error.unwrap_or("нет").to_string(),
    ));
    for (index, (title, value)) in rows.iter().enumerate() {
        let y = 150.0 + index as f32 * 50.0;
        text(
            &mut pixmap,
            fonts,
            (520.0, y),
            18.0,
            false,
            color(MUTED),
            title,
        );
        let ink = if title == "Ошибки" && value != "нет" {
            color(RED)
        } else {
            color(INK)
        };
        text(
            &mut pixmap,
            fonts,
            (520.0, y + 28.0),
            24.0,
            true,
            ink,
            value,
        );
    }

    let driver = engine.driver();
    for (hit, rect) in dash_pedals() {
        let amount = match hit {
            DashHit::Gas => driver.gas,
            DashHit::Brake => driver.brake,
            DashHit::Clutch => driver.clutch,
            _ => 0.0,
        };
        let ink = match hit {
            DashHit::Gas => color(AMBER),
            DashHit::Brake => color(RED),
            _ => color(0xc5ccd4),
        };
        fill_rect(&mut pixmap, rect, 16.0, color(PANEL));
        if amount > 0.02 {
            fill_rect(
                &mut pixmap,
                Rect {
                    x: rect.x,
                    y: rect.y + rect.h * (1.0 - amount),
                    w: rect.w,
                    h: rect.h * amount,
                },
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
        let label_ink = if amount > 0.55 {
            color(BACKGROUND)
        } else {
            color(INK)
        };
        text(
            &mut pixmap,
            fonts,
            (rect.x + 10.0, rect.y + rect.h - 22.0),
            16.0,
            false,
            label_ink,
            label,
        );
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
            if on { color(AMBER) } else { color(PANEL) },
        );
        let name = Gear::from_code(code).label();
        text(
            &mut pixmap,
            fonts,
            (rect.x + 26.0, rect.y + 40.0),
            24.0,
            true,
            if on { color(BACKGROUND) } else { color(INK) },
            name,
        );
    }
    button_caption(
        &mut pixmap,
        fonts,
        Rect {
            x: 760.0,
            y: 668.0,
            w: 180.0,
            h: 64.0,
        },
        "зажигание",
        !matches!(engine.motor(), crate::engine::Motor::Off),
    );
    button_caption(
        &mut pixmap,
        fonts,
        Rect {
            x: 960.0,
            y: 668.0,
            w: 180.0,
            h: 64.0,
        },
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
    if (Rect {
        x: 760.0,
        y: 668.0,
        w: 180.0,
        h: 64.0,
    })
    .contains(x, y)
    {
        return Some(DashHit::Ignition);
    }
    if (Rect {
        x: 960.0,
        y: 668.0,
        w: 180.0,
        h: 64.0,
    })
    .contains(x, y)
    {
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
    let mut row = [(
        DashHit::Gear(0),
        Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        },
    ); 7];
    for (index, hit) in keys.into_iter().enumerate() {
        row[index] = (
            hit,
            Rect {
                x: 40.0 + index as f32 * 84.0,
                y: 668.0,
                w: 72.0,
                h: 64.0,
            },
        );
    }
    row
}

fn button_caption(pixmap: &mut Pixmap, fonts: &mut Typeface, rect: Rect, label: &str, on: bool) {
    fill_rect(
        pixmap,
        rect,
        12.0,
        if on { color(AMBER) } else { color(PANEL) },
    );
    text(
        pixmap,
        fonts,
        (rect.x + 18.0, rect.y + 40.0),
        22.0,
        false,
        if on { color(BACKGROUND) } else { color(INK) },
        label,
    );
}

fn dash_road(pixmap: &mut Pixmap, speed: f32, reverse: bool) {
    let road = Rect {
        x: 40.0,
        y: 575.0,
        w: 460.0,
        h: 48.0,
    };
    fill_rect(pixmap, road, 8.0, color(PANEL));
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs_f32())
        .unwrap_or(0.0);
    let direction = if reverse { -1.0 } else { 1.0 };
    let shift = (seconds * speed.max(0.0) * 24.0 * direction).rem_euclid(70.0);
    let mut x = road.x - 70.0 + shift;
    while x < road.x + road.w {
        let mark = Rect {
            x,
            y: road.y + 20.0,
            w: 28.0,
            h: 6.0,
        };
        if mark.x + mark.w > road.x && mark.x < road.x + road.w {
            fill_rect(pixmap, mark, 0.0, color(AMBER));
        }
        x += 70.0;
    }
}

fn color(rgb: u32) -> Color {
    Color::from_rgba8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::CarMessage;

    #[test]
    fn disconnected_ecu_hides_stale_readings() {
        let mut fonts = Typeface::load();
        let empty = render(
            Screen::Ecu,
            &Climate::default(),
            &EcuView::default(),
            &mut fonts,
        );
        let mut stale = EcuView::default();
        stale.apply(&CarMessage::Coolant(110));
        stale.apply(&CarMessage::Errors(vec!["P0217".into()]));
        stale.apply(&CarMessage::Clutch(true));
        stale.linked = false;
        let disconnected = render(Screen::Ecu, &Climate::default(), &stale, &mut fonts);
        assert_eq!((disconnected.width(), disconnected.height()), (1024, 600));
        assert_eq!(empty.data(), disconnected.data());
    }

    #[test]
    fn readings_frames_differ_between_the_two_firmwares() {
        let mut fonts = Typeface::load();
        let climate = Climate::default();
        let m74 = render(
            Screen::Readings,
            &climate,
            &linked("итэлма-м74"),
            &mut fonts,
        );
        let january = render(
            Screen::Readings,
            &climate,
            &linked("январь-7.2"),
            &mut fonts,
        );
        assert_eq!(m74.width(), 1024);
        assert_eq!(m74.height(), 600);
        assert_ne!(m74.data(), january.data());
        let silent = render(Screen::Readings, &climate, &EcuView::default(), &mut fonts);
        assert_ne!(silent.data(), m74.data());
    }

    #[test]
    fn dashboard_shows_both_firmwares_and_hits_the_pedals() {
        let mut fonts = Typeface::load();
        let m74 = render_dashboard(&Vehicle::new(), false, &mut fonts);
        let january = render_dashboard(
            &Vehicle::with_firmware(crate::firmware::find("январь-7.2").unwrap()),
            true,
            &mut fonts,
        );
        assert_eq!((m74.width(), m74.height()), (1200, 760));
        assert_ne!(m74.data(), january.data());
        assert_eq!(hit_dashboard(1140.0, 400.0), Some(DashHit::Gas));
        assert_eq!(hit_dashboard(1040.0, 400.0), Some(DashHit::Brake));
        assert_eq!(hit_dashboard(940.0, 400.0), Some(DashHit::Clutch));
        assert_eq!(hit_dashboard(80.0, 690.0), Some(DashHit::Gear(1)));
        assert_eq!(hit_dashboard(800.0, 690.0), Some(DashHit::Ignition));
        assert_eq!(hit_dashboard(1000.0, 690.0), Some(DashHit::Starter));
        assert_eq!(Gear::from_code(-1), Gear::Reverse);
        assert_eq!(Gear::Reverse.code(), -1);
    }

    fn linked(id: &str) -> EcuView {
        let mut ecu = EcuView::default();
        ecu.linked = true;
        ecu.firmware = id.to_string();
        ecu.calibration = crate::firmware::find(id).unwrap().calibration.to_string();
        ecu.readings.push(("обороты".into(), 860.0));
        ecu
    }
}
