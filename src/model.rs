use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::firmware;
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
    pub automatic: bool,
    pub door: f32,
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
            automatic: false,
            door: 0.5,
        }
    }
}

impl Climate {
    pub fn boot_events(&self) -> Vec<OutputEvent> {
        vec![
            event("0", bool_value(self.recirculation)),
            event("1", self.commanded_door()),
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
        if !self.automatic {
            self.door = snapped;
        }
        Some(event("1", self.commanded_door()))
    }

    pub fn commanded_door(&self) -> f32 {
        if self.automatic {
            self.door
        } else {
            self.temperature
        }
    }

    /// Ручное касание возвращает заслонку к заданию ползунка.
    pub fn leave_auto(&mut self) -> Option<OutputEvent> {
        if !self.automatic {
            return None;
        }
        self.automatic = false;
        self.door = self.temperature;
        Some(event("1", self.temperature))
    }

    pub fn toggle_automatic(&mut self) {
        self.automatic = !self.automatic;
        if self.automatic {
            self.door = self.temperature;
        }
    }

    /// В режиме «климат» заслонка идёт к положению заданной температуры,
    /// с небольшой добавкой от ошибки. Скорость заслонки ограничена.
    pub fn regulate(&mut self, cabin_c: f32, dt: f32) -> Vec<OutputEvent> {
        if !self.automatic {
            return Vec::new();
        }
        let dt = dt.clamp(0.0, 1.0);
        let target = celsius_from_unit(self.temperature) as f32;
        let error = target - cabin_c;
        let desired =
            (crate::vehicle::Vehicle::door_for_cabin(target) + error * 0.004).clamp(0.0, 1.0);
        let step = 0.04 * dt;
        let delta = desired - self.door;
        let door = if delta.abs() <= step {
            desired
        } else {
            self.door + step.copysign(delta)
        };
        let mut events = Vec::new();
        if (door - self.door).abs() > 0.0001 {
            self.door = door;
            events.push(event("1", door));
        }
        let speed = if error.abs() > 4.0 {
            3
        } else if error.abs() > 1.2 {
            2
        } else {
            1
        };
        if (self.fan / MIN_FAN).round() as u8 != speed {
            events.extend(self.set_fan(speed));
        }
        events
    }

    /// Выключено и четыре фиксированные скорости; ноль снимает запрос кондиционера.
    pub fn set_fan(&mut self, speed: u8) -> Vec<OutputEvent> {
        let value = speed.min(4) as f32 * MIN_FAN;
        let mut events = Vec::new();
        if self.fan != value {
            self.fan = value;
            events.push(event("вентилятор", value));
        }
        if value == 0.0 && self.air_conditioner {
            self.air_conditioner = false;
            events.push(event("кондиционер", 0.0));
        }
        events
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
                "fan" => {
                    climate.set_fan((parse_unit(value) / MIN_FAN).round() as u8);
                }
                // Кондиционер после каждой загрузки выключен, даже если строка осталась в файле.
                "air_conditioner" => {}
                "automatic" => climate.automatic = parse_bool(value),
                _ => {}
            }
        }
        climate.air_conditioner = false;
        climate.door = climate.temperature;
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
        push_bool(&mut body, "automatic", self.automatic);
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

pub struct Journal;

impl Journal {
    pub fn line(&mut self, text: &str) {
        println!("{text}");
        let _ = std::io::stdout().flush();
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
    Readings,
    Drive,
}

impl Screen {
    fn next(self) -> Self {
        match self {
            Screen::Climate => Screen::Ecu,
            Screen::Ecu => Screen::Readings,
            Screen::Readings => Screen::Drive,
            Screen::Drive => Screen::Climate,
        }
    }

    fn previous(self) -> Self {
        match self {
            Screen::Climate => Screen::Drive,
            Screen::Ecu => Screen::Climate,
            Screen::Readings => Screen::Ecu,
            Screen::Drive => Screen::Readings,
        }
    }
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
    last_regulate: Instant,
}

pub struct EcuView {
    pub coolant_c: Option<i32>,
    pub errors: Vec<String>,
    pub clutch: Option<bool>,
    pub firmware: String,
    pub calibration: String,
    pub readings: Vec<(String, f32)>,
    pub cabin_c: Option<f32>,
    pub motor: String,
    pub gear: String,
    pub pedals: Option<(f32, f32, f32)>,
    pub linked: bool,
    heard: Option<Instant>,
}

impl Default for EcuView {
    fn default() -> Self {
        Self {
            coolant_c: None,
            errors: Vec::new(),
            clutch: None,
            firmware: String::new(),
            calibration: String::new(),
            readings: Vec::new(),
            cabin_c: None,
            motor: String::new(),
            gear: String::new(),
            pedals: None,
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
            CarMessage::Firmware(id) => {
                if self.firmware == *id {
                    false
                } else {
                    self.firmware.clone_from(id);
                    self.calibration.clear();
                    self.readings.clear();
                    true
                }
            }
            CarMessage::Calibration(text) => {
                if self.calibration == *text {
                    false
                } else {
                    self.calibration.clone_from(text);
                    true
                }
            }
            CarMessage::Reading(key, value) => self.store_reading(key, *value),
            CarMessage::Cabin(value) => self.cabin_c.replace(*value) != Some(*value),
            CarMessage::Motor(state) => {
                if self.motor == *state {
                    false
                } else {
                    self.motor.clone_from(state);
                    true
                }
            }
            CarMessage::Gear(gear) => {
                if self.gear == *gear {
                    false
                } else {
                    self.gear.clone_from(gear);
                    true
                }
            }
            CarMessage::Pedals(gas, brake, clutch) => {
                let next = Some((*gas, *brake, *clutch));
                if self.pedals == next {
                    false
                } else {
                    self.pedals = next;
                    true
                }
            }
        };
        link_changed || data_changed
    }

    fn store_reading(&mut self, key: &str, value: f32) -> bool {
        let Some(firmware) = firmware::find(&self.firmware) else {
            return false;
        };
        let Some(channel) = firmware.channel(key) else {
            return false;
        };
        let shown = firmware::format_value(channel, value);
        if let Some(slot) = self.readings.iter_mut().find(|(stored, _)| stored == key) {
            let previous = firmware::format_value(channel, slot.1);
            slot.1 = value;
            previous != shown
        } else {
            self.readings.push((key.to_string(), value));
            true
        }
    }

    pub fn refresh_link(&mut self) -> bool {
        let linked = self
            .heard
            .is_some_and(|heard| heard.elapsed() < LINK_TIMEOUT);
        if linked == self.linked {
            return false;
        }
        self.linked = linked;
        if !linked {
            self.coolant_c = None;
            self.errors.clear();
            self.clutch = None;
            self.firmware.clear();
            self.calibration.clear();
            self.readings.clear();
            self.cabin_c = None;
            self.motor.clear();
            self.gear.clear();
            self.pedals = None;
        }
        true
    }

    pub fn reading_of(&self, key: &str) -> Option<f32> {
        self.readings
            .iter()
            .find(|(stored, _)| stored == key)
            .map(|(_, value)| *value)
    }

    /// Строки экрана показаний: только каналы текущей прошивки.
    /// `None`, пока связи нет или имя прошивки панели неизвестно.
    pub fn reading_rows(&self) -> Option<Vec<(&'static str, String)>> {
        if !self.linked {
            return None;
        }
        let firmware = firmware::find(&self.firmware)?;
        Some(
            firmware
                .channels
                .iter()
                .map(|channel| {
                    let value = self
                        .readings
                        .iter()
                        .find(|(key, _)| key == channel.key)
                        .map(|(_, stored)| shown_reading(channel, *stored))
                        .unwrap_or_else(|| "нет данных".to_string());
                    (channel.title, value)
                })
                .collect(),
        )
    }

    pub fn readings_heading(&self) -> Option<String> {
        if !self.linked {
            return None;
        }
        let firmware = firmware::find(&self.firmware)?;
        let calibration = if self.calibration.is_empty() {
            firmware.calibration
        } else {
            self.calibration.as_str()
        };
        Some(format!("{}  {calibration}", firmware.family))
    }
}

fn shown_reading(channel: &firmware::Channel, value: f32) -> String {
    let number = firmware::format_value(channel, value);
    if channel.unit.is_empty() {
        number
    } else {
        format!("{number} {}", channel.unit)
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
        let mut journal = Journal;
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
            last_regulate: Instant::now(),
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
        if self.climate.automatic {
            if let Some(cabin) = self.ecu.cabin_c {
                let dt = self.last_regulate.elapsed().as_secs_f32();
                self.last_regulate = Instant::now();
                let events = self.climate.regulate(cabin, dt);
                if !events.is_empty() {
                    for event in &events {
                        self.publish(event);
                    }
                    dirty = true;
                }
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
                    self.screen = if dx < 0.0 {
                        self.screen.next()
                    } else {
                        self.screen.previous()
                    };
                    *fired = true;
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
        if let Some(event) = self.climate.leave_auto() {
            self.publish(&event);
            self.persist();
        }
        let Some(event) = self.climate.set_temperature(value) else {
            return false;
        };
        let _ = event;
        true
    }

    fn apply_hit(&mut self, hit: crate::layout::Hit) {
        if hit == crate::layout::Hit::Auto {
            self.climate.toggle_automatic();
            self.persist();
            return;
        }
        let mut events = Vec::new();
        if let Some(event) = self.climate.leave_auto() {
            events.push(event);
        }
        events.extend(match hit {
            crate::layout::Hit::Face => vec![self.climate.toggle_face()],
            crate::layout::Hit::Feet => vec![self.climate.toggle_feet()],
            crate::layout::Hit::Glass => vec![self.climate.toggle_glass()],
            crate::layout::Hit::Recirc => vec![self.climate.toggle_recirculation()],
            crate::layout::Hit::Ac => self.climate.toggle_air_conditioner(),
            crate::layout::Hit::Fan(speed) => self.climate.set_fan(speed),
            crate::layout::Hit::Slider | crate::layout::Hit::Auto => return,
        });
        if !events.is_empty() {
            for event in &events {
                self.publish(event);
            }
            self.persist();
        }
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
                CarMessage::Firmware(id) => {
                    self.journal.line(&format!("эбу прошивка {id}"));
                }
                CarMessage::Calibration(text) => {
                    self.journal.line(&format!("эбу калибровка {text}"));
                }
                CarMessage::Reading(_, _)
                | CarMessage::Cabin(_)
                | CarMessage::Motor(_)
                | CarMessage::Gear(_)
                | CarMessage::Pedals(_, _, _) => {}
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
    if let Some(dir) = std::env::var_os("OPENVAZ_STATE_DIR") {
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
    fn session_keeps_only_settings_in_the_state_directory() {
        let dir = std::env::temp_dir().join(format!("openvaz-journal-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        {
            let mut session = Session::boot(Some(dir.clone()));
            for _ in 0..1000 {
                session.apply_hit(crate::layout::Hit::Face);
                session.note_car(CarMessage::Coolant(90));
            }
        }
        let entries: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("settings")]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stopping_the_fan_also_turns_off_the_air_conditioner() {
        let mut climate = Climate::default();
        climate.toggle_air_conditioner();
        assert_eq!(
            climate.set_fan(0),
            vec![event("вентилятор", 0.0), event("кондиционер", 0.0)]
        );
        assert!(!climate.air_conditioner);
        assert_eq!(climate.fan, 0.0);
        assert!(climate.set_fan(0).is_empty());
        assert_eq!(climate.set_fan(255), vec![event("вентилятор", 1.0)]);
    }

    #[test]
    fn fan_buttons_persist_all_five_speeds() {
        let dir = std::env::temp_dir().join(format!("openvaz-fan-{}", std::process::id()));
        let mut session = Session::boot(Some(dir.clone()));
        session.apply_hit(crate::layout::Hit::Ac);
        for (index, (_, rect)) in crate::layout::fan_buttons().into_iter().enumerate() {
            session.pointer_down(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
            session.pointer_up();
            let expected = index as f32 * MIN_FAN;
            assert_eq!(session.climate.fan, expected);
            assert!(!session.climate.air_conditioner);
            assert_eq!(Session::boot(Some(dir.clone())).climate.fan, expected);
        }
        fs::write(dir.join("settings"), "fan=0.61\n").unwrap();
        assert_eq!(Climate::load(&dir.join("settings")).fan, 0.5);
        fs::remove_dir_all(dir).unwrap();
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
        ecu.cabin_c = Some(21.0);
        ecu.motor = "работает".into();
        ecu.gear = "2".into();
        ecu.pedals = Some((0.2, 0.0, 1.0));
        ecu.heard = Instant::now().checked_sub(Duration::from_secs(4));
        assert!(ecu.refresh_link());
        assert!(ecu.cabin_c.is_none());
        assert!(ecu.motor.is_empty());
        assert!(ecu.gear.is_empty());
        assert!(ecu.pedals.is_none());
    }

    #[test]
    fn a_reading_is_kept_only_when_the_firmware_has_that_key() {
        let mut ecu = EcuView::default();
        assert!(ecu.apply(&CarMessage::Coolant(40)));
        assert!(!ecu.apply(&CarMessage::Reading("кислород2".into(), 0.6)));
        assert!(ecu.readings.is_empty());
        assert!(ecu.apply(&CarMessage::Firmware("итэлма-м74".into())));
        assert!(ecu.apply(&CarMessage::Reading("кислород2".into(), 0.68)));
        assert_eq!(ecu.readings.len(), 1);
        assert!(ecu.apply(&CarMessage::Firmware("январь-7.2".into())));
        assert!(ecu.readings.is_empty());
        assert!(!ecu.apply(&CarMessage::Reading("кислород2".into(), 0.68)));
        assert!(ecu.readings.is_empty());
        ecu.heard = Instant::now().checked_sub(Duration::from_secs(4));
        assert!(ecu.refresh_link());
        assert!(ecu.firmware.is_empty());
        assert!(ecu.reading_rows().is_none());
    }

    #[test]
    fn readings_list_only_the_channels_of_the_selected_firmware() {
        let mut ecu = EcuView::default();
        ecu.linked = true;
        ecu.firmware = "январь-7.2".into();
        ecu.readings.push(("кислород2".into(), 0.68));
        ecu.readings.push(("обороты".into(), 850.0));
        let rows = ecu.reading_rows().unwrap();
        assert!(rows.iter().any(|(title, value)| *title == "Обороты" && value == "850 об/мин"));
        assert!(rows.iter().all(|(title, _)| *title != "Второй датчик кислорода"));
        assert!(rows.iter().any(|(_, value)| value == "нет данных"));
        ecu.firmware = "итэлма-м74".into();
        let m74 = ecu.reading_rows().unwrap();
        assert!(m74.len() > rows.len());
        assert!(m74.iter().any(|(title, value)| {
            *title == "Второй датчик кислорода" && value == "0.68 В"
        }));
    }

    #[test]
    fn swipe_walks_climate_ecu_and_readings() {
        let mut session = Session::boot(None);
        assert_eq!(session.screen, Screen::Climate);
        swipe(&mut session, -120.0);
        assert_eq!(session.screen, Screen::Ecu);
        swipe(&mut session, -120.0);
        assert_eq!(session.screen, Screen::Readings);
        swipe(&mut session, -120.0);
        assert_eq!(session.screen, Screen::Drive);
        swipe(&mut session, -120.0);
        assert_eq!(session.screen, Screen::Climate);
        swipe(&mut session, 120.0);
        assert_eq!(session.screen, Screen::Drive);
    }

    #[test]
    fn automatic_climate_reaches_the_setpoint_without_hunting() {
        let mut climate = Climate::default();
        climate.toggle_automatic();
        assert_eq!(celsius_from_unit(climate.temperature), 22);
        let mut cabin = 12.0;
        let mut previous = climate.door;
        let mut reversals = 0;
        let mut direction = 0.0;
        for _ in 0..400 {
            let _ = climate.regulate(cabin, 0.5);
            let delta = climate.door - previous;
            if delta.abs() > 0.0001 {
                let sign = delta.signum();
                if direction != 0.0 && sign != direction {
                    reversals += 1;
                }
                direction = sign;
            }
            previous = climate.door;
            let supply = crate::vehicle::Vehicle::cabin_supply(climate.door);
            let mix = ((0.02 + climate.fan * 0.12) * 0.5).min(0.4);
            cabin += (supply - cabin) * mix;
        }
        assert!((cabin - 22.0).abs() < 1.0, "салон {cabin}, заслонка {}", climate.door);
        assert!(reversals <= 2, "разворотов заслонки {reversals}");
        climate.leave_auto();
        assert!(!climate.automatic);
    }

    fn swipe(session: &mut Session, dx: f32) {
        session.pointer_down(900.0, 250.0);
        session.pointer_move(900.0 + dx, 250.0);
        session.pointer_up();
    }
}
