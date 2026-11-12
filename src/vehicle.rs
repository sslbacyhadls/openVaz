use crate::engine::{Driver, Engine};
use crate::firmware::{self, Firmware};

/// Исполнительные устройства автомобиля: заслонки догоняют команду,
/// вентилятор раскручивается с запаздыванием, муфта кондиционера
/// включается только при работающем вентиляторе и неперегретом моторе.
/// Мотор и скорость живут в `engine`: эмулятор заменяет машину.
pub struct Vehicle {
    target: [f32; 5],
    position: [f32; 5],
    fan_cmd: f32,
    fan: f32,
    ac_request: bool,
    pub clutch: bool,
    pub coolant: f32,
    pub error: Option<&'static str>,
    radiator_fan: bool,
    engine: Engine,
}

pub const DAMPERS: [&str; 5] = ["рециркуляция", "температура", "лицо", "ноги", "стекло"];

const SERVO_PER_SEC: f32 = 0.7;
const FAN_PER_SEC: f32 = 0.5;

impl Vehicle {
    pub fn new() -> Self {
        Self::with_firmware(firmware::default_firmware())
    }

    pub fn with_firmware(firmware: &'static Firmware) -> Self {
        Self {
            target: [0.0; 5],
            position: [0.0; 5],
            fan_cmd: 0.0,
            fan: 0.0,
            ac_request: false,
            clutch: false,
            coolant: 40.0,
            error: None,
            radiator_fan: false,
            engine: Engine::new(firmware),
        }
    }

    pub fn apply(&mut self, channel: &str, value: f32) {
        let value = value.clamp(0.0, 1.0);
        match channel {
            "0" => self.target[0] = value,
            "1" => self.target[1] = value,
            "2" => self.target[2] = value,
            "3" => self.target[3] = value,
            "4" => self.target[4] = value,
            "вентилятор" => self.fan_cmd = value,
            "кондиционер" => self.ac_request = value >= 0.5,
            _ => {}
        }
    }

    pub fn tick(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 1.0);
        for index in 0..5 {
            self.position[index] = slew(self.position[index], self.target[index], SERVO_PER_SEC * dt);
        }
        self.fan = slew(self.fan, self.fan_cmd, FAN_PER_SEC * dt);
        let goal = if self.engine.driver().ignition {
            let heat = if self.clutch { 8.0 } else { 0.0 };
            (90.0 - self.fan * 10.0 + heat).clamp(40.0, 110.0)
        } else {
            25.0
        };
        let blend = (0.12 * dt).min(1.0);
        self.coolant += (goal - self.coolant) * blend;
        self.coolant = self.coolant.clamp(10.0, 120.0);
        self.update_radiator_fan();
        self.clutch = self.ac_request && self.fan >= 0.2 && self.coolant < 105.0;
        let code = self.engine.firmware().overheat_code;
        self.error = if self.coolant >= 105.0 { Some(code) } else { None };
        let ac_load = self.clutch;
        self.engine.tick(dt, ac_load);
    }

    pub fn set_driver(&mut self, driver: Driver) {
        self.engine.set_driver(driver);
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn firmware(&self) -> &'static Firmware {
        self.engine.firmware()
    }

    pub fn reading(&self, key: &str) -> f32 {
        self.engine
            .value(key, self.coolant, self.radiator_fan, self.clutch)
    }

    pub fn wire_lines(&self) -> Vec<String> {
        let firmware = self.firmware();
        let mut lines = vec![
            format!("прошивка {}", firmware.id),
            format!("калибровка {}", firmware.calibration),
        ];
        for channel in firmware.channels {
            let text = firmware::format_value(channel, self.reading(channel.key));
            lines.push(format!("показание {} {text}", channel.key));
        }
        lines
    }

    fn update_radiator_fan(&mut self) {
        let firmware = self.engine.firmware();
        if self.coolant >= firmware.fan_on {
            self.radiator_fan = true;
        } else if self.coolant <= firmware.fan_off {
            self.radiator_fan = false;
        }
    }

    pub fn damper_command(&self, index: usize) -> f32 {
        self.target[index]
    }

    pub fn damper_position(&self, index: usize) -> f32 {
        self.position[index]
    }

    pub fn fan_command(&self) -> f32 {
        self.fan_cmd
    }

    pub fn fan_speed(&self) -> f32 {
        self.fan
    }

    pub fn ac_request(&self) -> bool {
        self.ac_request
    }

    pub fn status_lines(&self) -> [String; 3] {
        let coolant = self.coolant.round() as i32;
        let errors = self.error.unwrap_or("нет");
        let clutch = if self.clutch { "1" } else { "0" };
        [
            format!("охлаждение {coolant}"),
            format!("ошибки {errors}"),
            format!("муфта {clutch}"),
        ]
    }
}

impl Default for Vehicle {
    fn default() -> Self {
        Self::new()
    }
}

fn slew(current: f32, target: f32, max_step: f32) -> f32 {
    let delta = target - current;
    if delta.abs() <= max_step {
        target
    } else {
        current + max_step.copysign(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damper_follows_the_command() {
        let mut vehicle = Vehicle::new();
        vehicle.apply("1", 1.0);
        vehicle.tick(0.1);
        let position = vehicle.damper_position(1);
        assert!(position > 0.05 && position < 0.2);
        for _ in 0..30 {
            vehicle.tick(0.1);
        }
        assert!((vehicle.damper_position(1) - 1.0).abs() < 0.001);
    }

    #[test]
    fn clutch_waits_for_the_fan() {
        let mut vehicle = Vehicle::new();
        vehicle.apply("кондиционер", 1.0);
        vehicle.apply("вентилятор", 1.0);
        vehicle.tick(0.1);
        assert!(!vehicle.clutch);
        vehicle.tick(0.5);
        assert!(vehicle.fan_speed() >= 0.2);
        assert!(vehicle.clutch);
        assert!(vehicle.coolant < 105.0);
        assert!(vehicle.error.is_none());
    }

    #[test]
    fn coolant_warms_toward_operating_temperature() {
        let mut vehicle = Vehicle::new();
        let start = vehicle.coolant;
        for _ in 0..50 {
            vehicle.tick(0.2);
        }
        assert!(vehicle.coolant > start + 20.0);
        assert!(vehicle.coolant < 95.0);
        assert_eq!(vehicle.status_lines()[1], "ошибки нет");
        assert_eq!(vehicle.status_lines()[2], "муфта 0");
    }

    #[test]
    fn each_firmware_sends_only_its_own_keys_and_its_overheat_code() {
        for id in ["итэлма-м74", "январь-7.2"] {
            let firmware = firmware::find(id).unwrap();
            let mut vehicle = Vehicle::with_firmware(firmware);
            let lines = vehicle.wire_lines();
            assert_eq!(lines[0], format!("прошивка {}", firmware.id));
            assert_eq!(lines[1], format!("калибровка {}", firmware.calibration));
            let keys: Vec<&str> = lines
                .iter()
                .skip(2)
                .map(|line| line.split_whitespace().nth(1).unwrap())
                .collect();
            let expected: Vec<&str> = firmware.channels.iter().map(|channel| channel.key).collect();
            assert_eq!(keys, expected);
            vehicle.coolant = 110.0;
            vehicle.tick(0.0);
            assert_eq!(vehicle.error, Some(firmware.overheat_code));
        }
        let m74 = Vehicle::with_firmware(firmware::find("итэлма-м74").unwrap());
        let january = Vehicle::with_firmware(firmware::find("январь-7.2").unwrap());
        assert!(m74.wire_lines().iter().any(|line| line.contains("кислород2")));
        assert!(!january.wire_lines().iter().any(|line| line.contains("кислород2")));
    }
}
