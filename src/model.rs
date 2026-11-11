use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::link::{CarMessage, CarPort, LINK_TIMEOUT};

pub const PROFILE_ID: &str = "kalina";
pub const PROFILE_TITLE: &str = "Kalina";

/// Нижняя ненулевая скорость вентилятора: четыре рабочих положения плюс выключено.
pub const MIN_FAN: f32 = 0.25;

/// Заданная температура салона, которую выбирает ползунок.
pub const TEMP_MIN_C: i32 = 16;
pub const TEMP_MAX_C: i32 = 28;

pub fn celsius_from_unit(unit: f32) -> i32 {
    let span = (TEMP_MAX_C - TEMP_MIN_C) as f32;
    (TEMP_MIN_C as f32 + unit.clamp(0.0, 1.0) * span).round() as i32
}

pub fn unit_from_celsius(degrees: i32) -> f32 {
    let degrees = degrees.clamp(TEMP_MIN_C, TEMP_MAX_C);
    (degrees - TEMP_MIN_C) as f32 / (TEMP_MAX_C - TEMP_MIN_C) as f32
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputEvent {
    pub channel: &'static str,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Climate {
    pub recirculation: bool,
    pub temperature: f32,
    pub face: bool,
    pub feet: bool,
    pub glass: bool,
    pub fan: f32,
    pub air_conditioner: bool,
}

impl Default for Climate {
    fn default() -> Self {
        Self {
            recirculation: false,
            temperature: 0.5,
            face: false,
            feet: false,
            glass: false,
            fan: 0.0,
            air_conditioner: false,
        }
    }
}

impl Climate {
    pub fn boot_events(&self) -> Vec<OutputEvent> {
        vec![
            event("0", bool_value(self.recirculation)),
            event("1", self.temperature),
            event("2", bool_value(self.face)),
            event("3", bool_value(self.feet)),
            event("4", bool_value(self.glass)),
            event("вентилятор", self.fan),
            event("кондиционер", bool_value(self.air_conditioner)),
        ]
    }

    pub fn toggle_recirculation(&mut self) -> OutputEvent {
        self.recirculation = !self.recirculation;
        event("0", bool_value(self.recirculation))
    }

    pub fn toggle_face(&mut self) -> OutputEvent {
        self.face = !self.face;
        event("2", bool_value(self.face))
    }

    pub fn toggle_feet(&mut self) -> OutputEvent {
        self.feet = !self.feet;
        event("3", bool_value(self.feet))
    }

    pub fn toggle_glass(&mut self) -> OutputEvent {
        self.glass = !self.glass;
        event("4", bool_value(self.glass))
    }

    pub fn set_temperature(&mut self, value: f32) -> Option<OutputEvent> {
        let snapped = unit_from_celsius(celsius_from_unit(value));
        if (snapped - self.temperature).abs() < 0.001 {
            return None;
        }
        self.temperature = snapped;
        Some(event("1", self.temperature))
    }

    /// Запрос уходит в ЭБУ только при уже включённом вентиляторе.
    /// Если вентилятор выключен, сначала включается его нижняя скорость.
    pub fn toggle_air_conditioner(&mut self) -> Vec<OutputEvent> {
        if self.air_conditioner {
            self.air_conditioner = false;
            return vec![event("кондиционер", 0.0)];
        }
        let mut events = Vec::new();
        if self.fan <= 0.0 {
            self.fan = MIN_FAN;
            events.push(event("вентилятор", self.fan));
        }
        self.air_conditioner = true;
        events.push(event("кондиционер", 1.0));
        events
    }

    pub fn load(path: &Path) -> Self {
        let mut climate = Self::default();
        let Ok(text) = fs::read_to_string(path) else {
            return climate;
        };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "recirculation" => climate.recirculation = parse_bool(value),
                "temperature" => climate.temperature = parse_unit(value),
                "face" => climate.face = parse_bool(value),
                "feet" => climate.feet = parse_bool(value),
                "glass" => climate.glass = parse_bool(value),
                "fan" => climate.fan = parse_unit(value),
                // Кондиционер после каждой загрузки выключен, даже если строка осталась в файле.
                "air_conditioner" => {}
                _ => {}
            }
        }
        climate.air_conditioner = false;
        climate
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut body = String::new();
        push_bool(&mut body, "recirculation", self.recirculation);
        push_unit(&mut body, "temperature", self.temperature);
        push_bool(&mut body, "face", self.face);
        push_bool(&mut body, "feet", self.feet);
        push_bool(&mut body, "glass", self.glass);
        push_unit(&mut body, "fan", self.fan);
        let tmp = path.with_extension("tmp");
        {
            let mut file = File::create(&tmp)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        // Каталог тоже нужно сбросить: иначе при выключении зажигания
        // на диске остаётся settings.tmp, а не само имя файла.
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

fn event(channel: &'static str, value: f32) -> OutputEvent {
    OutputEvent { channel, value }
}

fn bool_value(on: bool) -> f32 {
    if on { 1.0 } else { 0.0 }
}

fn parse_bool(value: &str) -> bool {
    matches!(value.trim(), "1" | "true")
}

fn parse_unit(value: &str) -> f32 {
    value.trim().parse::<f32>().unwrap_or(0.0).clamp(0.0, 1.0)
}

fn push_bool(body: &mut String, key: &str, value: bool) {
    body.push_str(key);
    body.push('=');
    body.push(if value { '1' } else { '0' });
    body.push('\n');
}

fn push_unit(body: &mut String, key: &str, value: f32) {
    body.push_str(key);
    body.push('=');
    body.push_str(&format!("{value:.2}"));
    body.push('\n');
}

pub struct Journal {
    file: Option<File>,
}

impl Journal {
    pub fn open(dir: Option<&Path>) -> Self {
        let file = dir.and_then(|dir| {
            fs::create_dir_all(dir).ok()?;
            let path = dir.join("journal.log");
            File::options().create(true).append(true).open(path).ok()
        });
        Self { file }
    }

    pub fn line(&mut self, text: &str) {
        println!("{text}");
        let _ = std::io::stdout().flush();
        if let Some(file) = &mut self.file {
            let _ = writeln!(file, "{text}");
            let _ = file.flush();
        }
    }

    pub fn output(&mut self, event: &OutputEvent) {
        self.line(&format!(
            "выход {} значение {:.2}",
            event.channel, event.value
        ));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Climate,
    Ecu,
}

pub struct Session {
    pub climate: Climate,
    pub screen: Screen,
    pub ecu: EcuView,
    settings_path: Option<PathBuf>,
    journal: Journal,
    gesture: Gesture,
    car: Option<CarPort>,
    last_repeat: Instant,
}

pub struct EcuView {
    pub coolant_c: Option<i32>,
    pub errors: Vec<String>,
    pub clutch: Option<bool>,
    pub linked: bool,
    heard: Option<Instant>,
}

impl Default for EcuView {
    fn default() -> Self {
        Self {
            coolant_c: None,
            errors: Vec::new(),
            clutch: None,
            linked: false,
            heard: None,
        }
    }
}

impl EcuView {
    pub fn apply(&mut self, message: &CarMessage) -> bool {
        self.heard = Some(Instant::now());
        let link_changed = !self.linked;
        self.linked = true;
        let data_changed = match message {
            CarMessage::Coolant(value) => self.coolant_c.replace(*value) != Some(*value),
            CarMessage::Errors(list) => {
                if &self.errors == list {
                    false
                } else {
                    self.errors.clone_from(list);
                    true
                }
            }
            CarMessage::Clutch(on) => self.clutch.replace(*on) != Some(*on),
        };
        link_changed || data_changed
    }

    pub fn refresh_link(&mut self) -> bool {
        let linked = self
            .heard
            .is_some_and(|heard| heard.elapsed() < LINK_TIMEOUT);
        if linked == self.linked {
            return false;
        }
        self.linked = linked;
        true
    }
}

enum Gesture {
    Idle,
    Slider { changed: bool },
    Hold,
    Swipe { x0: f32, y0: f32, fired: bool },
}

impl Session {
    pub fn boot(state_dir: Option<PathBuf>) -> Self {
        let settings_path = state_dir.as_ref().map(|dir| dir.join("settings"));
        let mut climate = settings_path
            .as_deref()
            .map(Climate::load)
            .unwrap_or_default();
        climate.air_conditioner = false;
        let mut journal = Journal::open(state_dir.as_deref());
        journal.line(&format!("старт профиль={PROFILE_ID}"));
        journal.line("чтение положений: датчиков нет, команда по сохранённым значениям");
        for event in climate.boot_events() {
            journal.output(&event);
        }
        Self {
            climate,
            screen: Screen::Climate,
            ecu: EcuView::default(),
            settings_path,
            journal,
            gesture: Gesture::Idle,
            car: None,
            last_repeat: Instant::now(),
        }
    }

    pub fn car_fd(&self) -> Option<i32> {
        self.car.as_ref().map(CarPort::fd)
    }

    /// Читает ответы автомобиля и раз в секунду повторяет команды,
    /// чтобы эмулятор, подключившийся позже, узнал текущие задания.
    pub fn service_car(&mut self) -> bool {
        let mut dirty = false;
        if self.car.is_none() {
            self.attach_car();
        }
        if self.car.is_some() && self.last_repeat.elapsed() >= Duration::from_secs(1) {
            self.repeat_commands();
            self.last_repeat = Instant::now();
        }
        let polled = if let Some(car) = &mut self.car {
            car.poll_messages()
        } else {
            Ok(Vec::new())
        };
        if polled.is_err() {
            self.car = None;
            if self.ecu.linked {
                self.ecu.linked = false;
                self.ecu.heard = None;
                dirty = true;
            }
            self.journal.line("автомобиль отключён");
        }
        if let Ok(messages) = polled {
            for message in messages {
                dirty |= self.note_car(message);
            }
        }
        if self.ecu.refresh_link() {
            dirty = true;
            if !self.ecu.linked {
                self.journal.line("эбу нет связи");
            }
        }
        dirty
    }

    pub fn pointer_down(&mut self, x: f32, y: f32) {
        if self.screen == Screen::Climate {
            if let Some(hit) = crate::layout::hit_climate(x, y) {
                self.gesture = match hit {
                    crate::layout::Hit::Slider => {
                        let changed = self.apply_slider(x);
                        Gesture::Slider { changed }
                    }
                    other => {
                        self.apply_hit(other);
                        Gesture::Hold
                    }
                };
                return;
            }
        }
        self.gesture = Gesture::Swipe {
            x0: x,
            y0: y,
            fired: false,
        };
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) {
        match self.gesture {
            Gesture::Slider { .. } => {
                if self.apply_slider(x) {
                    if let Gesture::Slider { changed } = &mut self.gesture {
                        *changed = true;
                    }
                }
            }
            Gesture::Swipe {
                x0,
                y0,
                ref mut fired,
            } if !*fired => {
                let dx = x - x0;
                let dy = y - y0;
                if dx.abs() >= 72.0 && dx.abs() > dy.abs() * 1.2 {
                    if dx < 0.0 && self.screen == Screen::Climate {
                        self.screen = Screen::Ecu;
                        *fired = true;
                    } else if dx > 0.0 && self.screen == Screen::Ecu {
                        self.screen = Screen::Climate;
                        *fired = true;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn pointer_up(&mut self) {
        if let Gesture::Slider { changed: true } = self.gesture {
            let event = OutputEvent {
                channel: "1",
                value: self.climate.temperature,
            };
            self.publish(&event);
            self.persist();
        }
        self.gesture = Gesture::Idle;
    }

    fn apply_slider(&mut self, x: f32) -> bool {
        let track = crate::layout::slider_track();
        let value = ((x - track.x) / track.w).clamp(0.0, 1.0);
        let Some(event) = self.climate.set_temperature(value) else {
            return false;
        };
        let _ = event;
        true
    }

    fn apply_hit(&mut self, hit: crate::layout::Hit) {
        let events = match hit {
            crate::layout::Hit::Face => vec![self.climate.toggle_face()],
            crate::layout::Hit::Feet => vec![self.climate.toggle_feet()],
            crate::layout::Hit::Glass => vec![self.climate.toggle_glass()],
            crate::layout::Hit::Recirc => vec![self.climate.toggle_recirculation()],
            crate::layout::Hit::Ac => self.climate.toggle_air_conditioner(),
            crate::layout::Hit::Slider => return,
        };
        for event in &events {
            self.publish(event);
        }
        self.persist();
    }

    fn attach_car(&mut self) {
        let path = crate::link::car_path();
        let Ok(port) = CarPort::open(&path) else {
            return;
        };
        self.car = Some(port);
        self.journal
            .line(&format!("автомобиль подключён {}", path.display()));
        self.repeat_commands();
        self.last_repeat = Instant::now();
    }

    fn repeat_commands(&mut self) {
        let events = self.climate.boot_events();
        let mut failed = false;
        if let Some(car) = &mut self.car {
            for event in &events {
                if car.send_command(event.channel, event.value).is_err() {
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            self.car = None;
            self.journal.line("автомобиль отключён");
        }
    }

    fn publish(&mut self, event: &OutputEvent) {
        self.journal.output(event);
        let mut failed = false;
        if let Some(car) = &mut self.car {
            failed = car.send_command(event.channel, event.value).is_err();
        }
        if failed {
            self.car = None;
            self.journal.line("автомобиль отключён");
        }
    }

    fn note_car(&mut self, message: CarMessage) -> bool {
        let changed = self.ecu.apply(&message);
        if changed {
            match &message {
                CarMessage::Coolant(value) => {
                    self.journal.line(&format!("эбу охлаждение {value}"));
                }
                CarMessage::Errors(list) if list.is_empty() => {
                    self.journal.line("эбу ошибки нет");
                }
                CarMessage::Errors(list) => {
                    self.journal
                        .line(&format!("эбу ошибки {}", list.join(" ")));
                }
                CarMessage::Clutch(on) => {
                    self.journal
                        .line(&format!("эбу муфта {}", if *on { 1 } else { 0 }));
                }
            }
        }
        changed
    }

    fn persist(&mut self) {
        if let Some(path) = &self.settings_path {
            if let Err(error) = self.climate.save(path) {
                self.journal
                    .line(&format!("настройки не записаны: {error}"));
            }
        }
    }
}

pub fn writable_state_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("OPENVAS_STATE_DIR") {
        candidates.push(PathBuf::from(dir));
    }
    candidates.push(PathBuf::from("/var/lib/openvaz"));
    candidates.push(PathBuf::from("state"));
    candidates.into_iter().find(|dir| dir_is_writable(dir))
}

fn dir_is_writable(dir: &Path) -> bool {
    fs::create_dir_all(dir).is_ok() && {
        let probe = dir.join(".probe");
        match File::create(&probe) {
            Ok(_) => {
                let _ = fs::remove_file(probe);
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_conditioner_does_not_survive_reload_and_starts_the_fan() {
        let dir = std::env::temp_dir().join(format!("openvaz-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings");
        let mut climate = Climate::default();
        climate.face = true;
        climate.temperature = 0.2;
        let events = climate.toggle_air_conditioner();
        assert_eq!(events[0].channel, "вентилятор");
        assert_eq!(events[0].value, MIN_FAN);
        assert_eq!(events[1].channel, "кондиционер");
        climate.save(&path).unwrap();
        let loaded = Climate::load(&path);
        assert!(loaded.face);
        assert!((loaded.temperature - 0.2).abs() < 0.001);
        assert!((loaded.fan - MIN_FAN).abs() < 0.001);
        assert!(!loaded.air_conditioner);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn temperature_slider_uses_cabin_degrees() {
        assert_eq!(celsius_from_unit(0.0), TEMP_MIN_C);
        assert_eq!(celsius_from_unit(1.0), TEMP_MAX_C);
        assert_eq!(celsius_from_unit(0.5), 22);
        let mut climate = Climate::default();
        let event = climate.set_temperature(0.0).unwrap();
        assert_eq!(event.value, 0.0);
        assert_eq!(celsius_from_unit(climate.temperature), 16);
        assert!(climate.set_temperature(0.02).is_none());
        let event = climate.set_temperature(1.0).unwrap();
        assert!((event.value - 1.0).abs() < 0.001);
        assert_eq!(celsius_from_unit(climate.temperature), 28);
    }

    #[test]
    fn stored_air_conditioner_flag_is_ignored() {
        let dir = std::env::temp_dir().join(format!("openvaz-ac-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings");
        fs::write(&path, "air_conditioner=1\nface=1\n").unwrap();
        let loaded = Climate::load(&path);
        assert!(loaded.face);
        assert!(!loaded.air_conditioner);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn directions_are_independent() {
        let mut climate = Climate::default();
        climate.toggle_face();
        climate.toggle_feet();
        assert!(climate.face && climate.feet && !climate.glass);
    }

    #[test]
    fn ecu_link_drops_after_silence() {
        let mut ecu = EcuView::default();
        assert!(ecu.apply(&CarMessage::Coolant(80)));
        assert!(ecu.linked);
        assert!(!ecu.apply(&CarMessage::Coolant(80)));
        ecu.heard = Instant::now().checked_sub(Duration::from_secs(4));
        assert!(ecu.refresh_link());
        assert!(!ecu.linked);
        assert_eq!(
            ecu.apply(&CarMessage::Errors(vec!["P0217".into()])),
            true
        );
        assert_eq!(ecu.errors, vec!["P0217".to_string()]);
    }
}
