use crate::engine::{Driver, Engine, Gear, Motor};
use crate::firmware::{self, format_channel, Firmware};

/// Исполнительные устройства автомобиля: заслонки догоняют команду,
/// вентилятор раскручивается с запаздыванием, муфта кондиционера
/// включается только при работающем вентиляторе и неперегретом моторе.
/// Двигатель считается выбранной калибровкой Января 7.2.
pub struct Vehicle {
    target: [f32; 5],
    position: [f32; 5],
    fan_cmd: f32,
    fan: f32,
    ac_request: bool,
    pub clutch: bool,
    engine: Engine,
    /// Команды с сенсорного экрана.
    panel: Driver,
    /// Удержание клавиш. Пустая передача значит «клавиатура передачу не выбирала».
    keys: KeyInput,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct KeyInput {
    pub gas: f32,
    pub brake: f32,
    pub clutch: f32,
    pub gear: Option<Gear>,
    pub ignition: Option<bool>,
    pub crank: bool,
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
            engine: Engine::new(firmware),
            panel: Driver::default(),
            keys: KeyInput::default(),
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
        self.clutch = self.ac_request && self.fan >= 0.2 && self.coolant() < 105.0;
        self.engine.set_ac_load(self.clutch);
        self.engine.set_driver(self.combined_driver());
        self.engine.tick(dt);
    }

    pub fn set_panel_driver(&mut self, driver: Driver) {
        self.panel = driver;
    }

    pub fn panel_driver(&self) -> Driver {
        self.panel
    }

    pub fn apply_panel(&mut self, command: crate::link::DriverCommand) {
        match command {
            crate::link::DriverCommand::Gas(value) => self.panel.gas = value,
            crate::link::DriverCommand::Brake(value) => self.panel.brake = value,
            crate::link::DriverCommand::Clutch(value) => self.panel.clutch = value,
            crate::link::DriverCommand::Gear(code) => self.panel.gear = Gear::from_code(code),
            crate::link::DriverCommand::Ignition(on) => self.panel.ignition = on,
            crate::link::DriverCommand::Crank(on) => self.panel.crank = on,
        }
    }

    pub fn set_keys(&mut self, keys: KeyInput) {
        self.keys = keys;
    }

    fn combined_driver(&self) -> Driver {
        Driver {
            gas: self.panel.gas.max(self.keys.gas),
            brake: self.panel.brake.max(self.keys.brake),
            clutch: self.panel.clutch.max(self.keys.clutch),
            gear: self.keys.gear.unwrap_or(self.panel.gear),
            ignition: self.keys.ignition.unwrap_or(true) && self.panel.ignition,
            crank: self.panel.crank || self.keys.crank,
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

    pub fn coolant(&self) -> f32 {
        self.engine.coolant()
    }

    pub fn error(&self) -> Option<&'static str> {
        self.engine.overheat_code()
    }

    pub fn firmware(&self) -> &'static Firmware {
        self.engine.firmware()
    }

    pub fn motor(&self) -> Motor {
        self.engine.motor()
    }

    pub fn speed_kmh(&self) -> f32 {
        self.engine.value("скорость") * if self.engine.driver().gear == Gear::Reverse { -1.0 } else { 1.0 }
    }

    pub fn rpm(&self) -> f32 {
        self.engine.value("обороты")
    }

    pub fn channel(&self, key: &str) -> f32 {
        self.engine.value(key)
    }

    pub fn driver(&self) -> Driver {
        self.engine.driver()
    }

    pub fn status_lines(&self) -> [String; 3] {
        let coolant = self.coolant().round() as i32;
        let errors = self.error().unwrap_or("нет");
        let clutch = if self.clutch { "1" } else { "0" };
        [
            format!("охлаждение {coolant}"),
            format!("ошибки {errors}"),
            format!("муфта {clutch}"),
        ]
    }

    pub fn wire_lines(&self) -> Vec<String> {
        let driver = self.driver();
        let mut lines = vec![
            format!("прошивка {}", self.firmware().family),
            format!("калибровка {}", self.firmware().id),
        ];
        lines.extend(self.status_lines());
        for channel in self.firmware().channels() {
            let value = format_channel(channel, self.engine.value(channel.key));
            lines.push(format!("показание {} {value}", channel.key));
        }
        lines.push(format!("ход газ {:.2}", driver.gas));
        lines.push(format!("ход тормоз {:.2}", driver.brake));
        lines.push(format!("ход сцепление {:.2}", driver.clutch));
        lines.push(format!("ход передача {}", driver.gear.code()));
        lines.push(format!("ход состояние {}", self.motor().code()));
        lines
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
        assert!(vehicle.coolant() < 105.0);
        assert!(vehicle.error().is_none());
    }

    #[test]
    fn coolant_warms_toward_operating_temperature() {
        let mut vehicle = Vehicle::new();
        let start = vehicle.coolant();
        for _ in 0..50 {
            vehicle.tick(0.2);
        }
        assert!(vehicle.coolant() > start + 20.0);
        assert!(vehicle.coolant() < 95.0);
        assert_eq!(vehicle.status_lines()[1], "ошибки нет");
        assert_eq!(vehicle.status_lines()[2], "муфта 0");
    }
}
