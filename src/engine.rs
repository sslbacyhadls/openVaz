use crate::firmware::Firmware;

/// Ручная коробка Калины и переднеприводных ВАЗ: пять передач, задняя, нейтраль.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gear {
    Neutral,
    Reverse,
    Forward(u8),
}

impl Gear {
    pub fn from_code(code: i32) -> Self {
        match code {
            -1 => Gear::Reverse,
            1 => Gear::Forward(1),
            2 => Gear::Forward(2),
            3 => Gear::Forward(3),
            4 => Gear::Forward(4),
            5 => Gear::Forward(5),
            _ => Gear::Neutral,
        }
    }

    pub fn code(self) -> i32 {
        match self {
            Gear::Reverse => -1,
            Gear::Neutral => 0,
            Gear::Forward(gear) => i32::from(gear),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Gear::Neutral => "N",
            Gear::Reverse => "R",
            Gear::Forward(1) => "1",
            Gear::Forward(2) => "2",
            Gear::Forward(3) => "3",
            Gear::Forward(4) => "4",
            Gear::Forward(5) => "5",
            Gear::Forward(_) => "N",
        }
    }

    fn ratio(self) -> Option<f32> {
        match self {
            Gear::Neutral => None,
            Gear::Reverse => Some(3.53),
            Gear::Forward(1) => Some(3.636),
            Gear::Forward(2) => Some(1.950),
            Gear::Forward(3) => Some(1.357),
            Gear::Forward(4) => Some(0.941),
            Gear::Forward(5) => Some(0.784),
            Gear::Forward(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motor {
    Off,
    Cranking,
    Running,
    Stalled,
}

impl Motor {
    pub fn code(self) -> i32 {
        match self {
            Motor::Off => 0,
            Motor::Cranking => 1,
            Motor::Running => 2,
            Motor::Stalled => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Motor::Off => "зажигание выключено",
            Motor::Cranking => "стартер",
            Motor::Running => "работает",
            Motor::Stalled => "заглох",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Driver {
    pub gas: f32,
    pub brake: f32,
    pub clutch: f32,
    pub gear: Gear,
    pub ignition: bool,
    pub crank: bool,
}

impl Default for Driver {
    fn default() -> Self {
        Self {
            gas: 0.0,
            brake: 0.0,
            clutch: 0.0,
            gear: Gear::Neutral,
            ignition: true,
            crank: false,
        }
    }
}

const WHEEL_RADIUS: f32 = 0.289;
const MASS: f32 = 1150.0;
const AIR_DENSITY: f32 = 1.2;

pub struct Engine {
    firmware: &'static Firmware,
    driver: Driver,
    throttle: f32,
    rpm: f32,
    speed_kmh: f32,
    coolant: f32,
    radiator_fan: bool,
    ac_load: bool,
    rich_left: f32,
    o2_phase: f32,
    running: bool,
    stalled: bool,
}

impl Engine {
    pub fn new(firmware: &'static Firmware) -> Self {
        Self {
            firmware,
            driver: Driver::default(),
            throttle: 0.0,
            rpm: firmware.idle_rpm,
            speed_kmh: 0.0,
            coolant: 40.0,
            radiator_fan: false,
            ac_load: false,
            rich_left: 0.0,
            o2_phase: 0.0,
            running: true,
            stalled: false,
        }
    }

    pub fn firmware(&self) -> &'static Firmware {
        self.firmware
    }

    pub fn set_driver(&mut self, driver: Driver) {
        self.driver = driver;
        self.driver.gas = self.driver.gas.clamp(0.0, 1.0);
        self.driver.brake = self.driver.brake.clamp(0.0, 1.0);
        self.driver.clutch = self.driver.clutch.clamp(0.0, 1.0);
    }

    pub fn driver(&self) -> Driver {
        self.driver
    }

    pub fn set_ac_load(&mut self, on: bool) {
        self.ac_load = on && self.firmware.has_ac;
    }

    pub fn coolant(&self) -> f32 {
        self.coolant
    }

    pub fn tick(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.5);
        let gas = if self.driver.ignition { self.driver.gas } else { 0.0 };
        let plate = if self.firmware.electronic_throttle && self.running {
            let cold = ((70.0 - self.coolant).max(0.0) / 70.0 * 0.06).min(0.06);
            gas.max(0.05 + cold)
        } else {
            gas
        };
        self.throttle = approach(self.throttle, plate, 2.5 * dt);
        self.update_motor(dt);
        self.update_speed(dt);
        self.update_coolant(dt);
        if self.firmware.tip_out_rich && self.driver.gas < 0.12 && self.throttle > 0.35 {
            self.rich_left = 0.45;
        }
        self.rich_left = (self.rich_left - dt).max(0.0);
        if self.running {
            self.o2_phase += dt * (3.0 + self.rpm / 500.0);
        }
    }

    pub fn value(&self, key: &str) -> f32 {
        match key {
            "обороты" => self.rpm,
            "обороты_хх" => self.desired_idle(),
            "скорость" => self.speed_kmh.abs(),
            "педаль" => self.driver.gas * 100.0,
            "дроссель" => self.throttle * 100.0,
            "воздух" => self.intake_c(),
            "дмрв" => self.maf_kg_h(),
            "впрыск" => self.injection_ms(),
            "зажигание" => self.advance_deg(),
            "напряжение" => self.battery(),
            "коррекция" => self.injection_correction(),
            "кислород" => self.oxygen(),
            "кислород2" => self.rear_oxygen(),
            "рхх" => self.iac_steps(),
            "продувка" => self.purge(),
            "нагрузка" => self.load(),
            "расход_час" => self.fuel_lph(),
            "расход_путь" => self.fuel_l_per_100(),
            "фаза" => {
                if self.running && self.firmware.has_phase {
                    1.0
                } else {
                    0.0
                }
            }
            "детонация" => self.knock_retard(),
            _ => 0.0,
        }
    }

    pub fn motor(&self) -> Motor {
        if !self.driver.ignition && !self.driver.crank {
            Motor::Off
        } else if self.driver.crank && !self.running {
            Motor::Cranking
        } else if self.stalled && !self.running {
            Motor::Stalled
        } else if self.running {
            Motor::Running
        } else {
            Motor::Off
        }
    }

    pub fn overheat_code(&self) -> Option<&'static str> {
        if self.coolant >= 108.0 {
            Some("P0217")
        } else {
            None
        }
    }

    fn update_motor(&mut self, dt: f32) {
        if !self.driver.ignition && !self.driver.crank {
            self.running = false;
            self.rpm = approach(self.rpm, 0.0, 2200.0 * dt);
            return;
        }
        if self.driver.crank && !self.running {
            self.rpm = approach(self.rpm, 340.0, 700.0 * dt);
            if self.driver.ignition && self.rpm > 260.0 && !self.clutch_locked_at_rest() {
                self.running = true;
                self.stalled = false;
            }
        }
        if !self.running {
            self.rpm = approach(self.rpm, if self.driver.crank { 340.0 } else { 0.0 }, 1800.0 * dt);
            return;
        }
        let want = self.wanted_rpm();
        let lock = self.clutch_lock();
        if let Some(ratio) = self.driver.gear.ratio() {
            let road = rpm_from_speed(self.speed_kmh, ratio, self.firmware.final_drive);
            if lock > 0.82 {
                self.rpm = approach(self.rpm, road, 5000.0 * dt);
                if self.rpm < 520.0 && self.throttle < 0.35 {
                    self.running = false;
                    self.stalled = true;
                    self.rpm = approach(self.rpm, 0.0, 2500.0 * dt);
                }
            } else if lock > 0.08 {
                let mixed = want * (1.0 - lock) + road * lock;
                self.rpm = approach(self.rpm, mixed, 3200.0 * dt);
            } else {
                self.rpm = approach(self.rpm, want, 2200.0 * dt);
            }
        } else {
            self.rpm = approach(self.rpm, want, 2200.0 * dt);
        }
        if self.rpm > self.firmware.rev_limit {
            self.rpm = self.firmware.rev_limit;
        }
    }

    fn update_speed(&mut self, dt: f32) {
        let lock = self.clutch_lock();
        let drive = self
            .driver
            .gear
            .ratio()
            .map(|ratio| {
                let sign = if self.driver.gear == Gear::Reverse { -1.0 } else { 1.0 };
                sign * self.torque() * ratio * self.firmware.final_drive * lock / WHEEL_RADIUS
            })
            .unwrap_or(0.0);
        let speed_ms = self.speed_kmh / 3.6;
        let drag = 0.42 * speed_ms * speed_ms.abs() + if speed_ms.abs() > 0.15 { 140.0 * speed_ms.signum() } else { 0.0 };
        let brake = if speed_ms.abs() > 0.05 {
            self.driver.brake * 8500.0 * speed_ms.signum()
        } else if self.driver.brake > 0.2 {
            speed_ms * 4000.0
        } else {
            0.0
        };
        let accel = (drive - drag - brake) / MASS;
        self.speed_kmh += accel * 3.6 * dt;
        if self.speed_kmh.abs() < 0.15 && self.driver.brake > 0.15 {
            self.speed_kmh = 0.0;
        }
        self.speed_kmh = self.speed_kmh.clamp(-40.0, 180.0);
    }

    fn update_coolant(&mut self, dt: f32) {
        if self.radiator_fan {
            if self.coolant <= self.firmware.fan_off {
                self.radiator_fan = false;
            }
        } else if self.coolant >= self.firmware.fan_on {
            self.radiator_fan = true;
        }
        let goal = if self.running {
            78.0 + self.throttle * 26.0 + (self.rpm / self.firmware.rev_limit) * 10.0
                + if self.ac_load { 6.0 } else { 0.0 }
                - if self.radiator_fan { 14.0 } else { 0.0 }
        } else {
            22.0
        };
        let blend = (0.12 * dt).min(1.0);
        self.coolant += (goal - self.coolant) * blend;
        self.coolant = self.coolant.clamp(10.0, 120.0);
    }

    fn clutch_lock(&self) -> f32 {
        if self.driver.gear.ratio().is_none() || !self.running {
            return 0.0;
        }
        (1.0 - self.driver.clutch).clamp(0.0, 1.0).powf(1.4)
    }

    fn clutch_locked_at_rest(&self) -> bool {
        self.clutch_lock() > 0.8 && self.speed_kmh.abs() < 1.0 && self.driver.gas < 0.25
    }

    fn wanted_rpm(&self) -> f32 {
        let idle = self.desired_idle();
        let span = (self.firmware.rev_limit - idle).max(0.0);
        idle + self.throttle.powf(0.75) * span * 0.96
    }

    fn desired_idle(&self) -> f32 {
        self.firmware.idle_rpm + if self.ac_load { 100.0 } else { 0.0 }
    }

    fn torque(&self) -> f32 {
        if !self.running || self.rpm >= self.firmware.rev_limit - 40.0 {
            return 0.0;
        }
        let x = (self.rpm / 3400.0).clamp(0.2, 1.35);
        let shape = (1.2 * x - 0.38 * x * x).clamp(0.25, 1.0);
        (24.0 + self.throttle * self.firmware.peak_nm * shape - if self.ac_load { 8.0 } else { 0.0 }).max(0.0)
    }

    fn maf_kg_h(&self) -> f32 {
        if !self.running {
            return 0.0;
        }
        let fill = 0.22 + 0.78 * self.throttle;
        let per_second = self.firmware.displacement_l * (self.rpm / 120.0) / 1000.0;
        per_second * self.firmware.volumetric * fill * AIR_DENSITY * 3600.0
    }

    fn injection_ms(&self) -> f32 {
        if !self.running || self.rpm < 100.0 {
            return 0.0;
        }
        let fuel_gps = self.maf_kg_h() / 3.6 / 14.7;
        let events = self.rpm / 30.0;
        let mut pulse = fuel_gps / events / self.firmware.injector_gps * 1000.0;
        pulse *= self.injection_correction();
        pulse.clamp(0.0, 20.0)
    }

    fn injection_correction(&self) -> f32 {
        if self.rich_left > 0.0 {
            1.28
        } else {
            1.0
        }
    }

    fn advance_deg(&self) -> f32 {
        if !self.running {
            return 0.0;
        }
        (8.0 + self.rpm / 1000.0 * 3.2 + self.throttle * 8.0 - self.knock_retard()).clamp(4.0, 42.0)
    }

    fn knock_retard(&self) -> f32 {
        if self.firmware.has_knock && self.running && self.throttle > 0.72 && self.rpm > 3200.0 {
            1.5 + self.throttle * 2.0
        } else {
            0.0
        }
    }

    fn oxygen(&self) -> f32 {
        if !self.running {
            return 0.45;
        }
        if self.rich_left > 0.0 {
            return 0.84;
        }
        0.45 + self.o2_phase.sin() * 0.38
    }

    /// После катализатора напряжение почти ровное, пока мотор прогрет.
    fn rear_oxygen(&self) -> f32 {
        if !(self.running && self.coolant > 60.0) {
            return 0.45;
        }
        0.68 + self.o2_phase.sin() * 0.04
    }

    fn iac_steps(&self) -> f32 {
        if !self.running {
            return 130.0;
        }
        let cold = (70.0 - self.coolant).max(0.0) * 1.4;
        let ac = if self.ac_load { 28.0 } else { 0.0 };
        (36.0 + cold + ac).clamp(20.0, 180.0)
    }

    fn purge(&self) -> f32 {
        if self.running && self.coolant > 60.0 && self.throttle < 0.45 {
            12.0 + self.throttle * 30.0
        } else {
            0.0
        }
    }

    fn load(&self) -> f32 {
        if !self.running {
            return 0.0;
        }
        (self.throttle * 70.0 + self.rpm / self.firmware.rev_limit * 30.0).clamp(0.0, 100.0)
    }

    fn fuel_lph(&self) -> f32 {
        if !self.running {
            return 0.0;
        }
        let gps = self.maf_kg_h() / 3.6 / 14.7 * self.injection_correction();
        gps / 740.0 * 3600.0
    }

    fn fuel_l_per_100(&self) -> f32 {
        if self.speed_kmh.abs() < 5.0 {
            0.0
        } else {
            self.fuel_lph() / self.speed_kmh.abs() * 100.0
        }
    }

    fn intake_c(&self) -> f32 {
        22.0 + self.throttle * 10.0 + if self.running { 4.0 } else { 0.0 }
    }

    fn battery(&self) -> f32 {
        if self.driver.crank && !self.running {
            10.3
        } else if self.running {
            14.2 - if self.radiator_fan { 0.4 } else { 0.0 } - if self.ac_load { 0.25 } else { 0.0 }
        } else {
            12.3
        }
    }

    #[cfg(test)]
    pub fn force_coolant(&mut self, value: f32) {
        self.coolant = value;
        self.update_coolant(0.0);
    }

    #[cfg(test)]
    pub fn radiator_fan(&self) -> bool {
        self.radiator_fan
    }
}

fn rpm_from_speed(speed_kmh: f32, ratio: f32, final_drive: f32) -> f32 {
    let wheel_rpm = speed_kmh.abs() / 3.6 / (2.0 * std::f32::consts::PI * WHEEL_RADIUS) * 60.0;
    wheel_rpm * ratio * final_drive
}

fn approach(current: f32, target: f32, max_step: f32) -> f32 {
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
    use crate::firmware::find;

    fn engine(id: &str) -> Engine {
        Engine::new(find(id).unwrap())
    }

    fn drive(engine: &mut Engine, gas: f32, brake: f32, clutch: f32, gear: Gear, seconds: f32) {
        engine.set_driver(Driver {
            gas,
            brake,
            clutch,
            gear,
            ignition: true,
            crank: false,
        });
        let mut left = seconds;
        while left > 0.0 {
            let step = left.min(0.05);
            engine.tick(step);
            left -= step;
        }
    }

    #[test]
    fn neutral_gas_raises_rpm_to_the_firmware_limit() {
        let mut eight = engine("I201CO58");
        drive(&mut eight, 1.0, 0.0, 0.0, Gear::Neutral, 6.0);
        assert!(eight.value("обороты") > 5000.0);
        assert!(eight.value("обороты") <= 5600.0);
        let mut sixteen = engine("I205DO54");
        drive(&mut sixteen, 1.0, 0.0, 0.0, Gear::Neutral, 6.0);
        assert!(sixteen.value("обороты") > eight.value("обороты"));
        assert!(sixteen.value("обороты") <= 6200.0);
    }

    #[test]
    fn first_gear_picks_up_speed_and_brake_stops_it() {
        let mut car = engine("I201CO58");
        drive(&mut car, 0.7, 0.0, 0.45, Gear::Forward(1), 4.0);
        assert!(car.speed_kmh > 8.0, "скорость {}", car.speed_kmh);
        assert_eq!(car.motor(), Motor::Running);
        let moving = car.speed_kmh;
        drive(&mut car, 0.0, 1.0, 1.0, Gear::Forward(1), 3.0);
        assert!(car.speed_kmh < moving);
        assert!(car.speed_kmh < 2.0, "после тормоза {}", car.speed_kmh);
    }

    #[test]
    fn dropped_clutch_at_rest_stalls() {
        let mut car = engine("I204DO57");
        drive(&mut car, 0.0, 0.0, 0.0, Gear::Forward(1), 1.2);
        assert_eq!(car.motor(), Motor::Stalled);
        assert!(car.value("впрыск") < 0.1);
    }

    #[test]
    fn tip_out_on_dm53_goes_rich_and_do57_does_not() {
        let mut buggy = engine("I204DM53");
        drive(&mut buggy, 0.9, 0.0, 1.0, Gear::Neutral, 1.5);
        drive(&mut buggy, 0.0, 0.0, 1.0, Gear::Neutral, 0.15);
        let mut fixed = engine("I204DO57");
        drive(&mut fixed, 0.9, 0.0, 1.0, Gear::Neutral, 1.5);
        drive(&mut fixed, 0.0, 0.0, 1.0, Gear::Neutral, 0.15);
        assert!(buggy.value("коррекция") > 1.2);
        assert!(buggy.value("кислород") > 0.75);
        assert!((fixed.value("коррекция") - 1.0).abs() < 0.01);
    }

    #[test]
    fn kalina_fan_thresholds_differ() {
        let mut narrow = engine("I201CO57");
        narrow.force_coolant(100.0);
        assert!(narrow.radiator_fan());
        narrow.force_coolant(96.0);
        assert!(!narrow.radiator_fan());
        let mut split = engine("I201CO58");
        split.force_coolant(100.0);
        assert!(!split.radiator_fan());
        split.force_coolant(102.0);
        assert!(split.radiator_fan());
        split.force_coolant(96.0);
        assert!(split.radiator_fan());
        split.force_coolant(93.0);
        assert!(!split.radiator_fan());
    }

    #[test]
    fn starter_restarts_a_stalled_engine() {
        let mut car = engine("I226FM10");
        drive(&mut car, 0.0, 0.0, 0.0, Gear::Forward(1), 1.2);
        assert_eq!(car.motor(), Motor::Stalled);
        car.set_driver(Driver {
            gas: 0.2,
            clutch: 1.0,
            gear: Gear::Neutral,
            ignition: true,
            crank: true,
            brake: 0.0,
        });
        car.tick(1.0);
        assert_eq!(car.motor(), Motor::Running);
        assert!(car.value("обороты") > 400.0);
    }

    #[test]
    fn rear_oxygen_on_21126_stays_steady() {
        let mut car = engine("I464SG04");
        drive(&mut car, 0.0, 0.0, 1.0, Gear::Neutral, 20.0);
        let mut front_min = 1.0_f32;
        let mut front_max = 0.0_f32;
        let mut rear_min = 1.0_f32;
        let mut rear_max = 0.0_f32;
        for _ in 0..40 {
            car.tick(0.05);
            let front = car.value("кислород");
            let rear = car.value("кислород2");
            front_min = front_min.min(front);
            front_max = front_max.max(front);
            rear_min = rear_min.min(rear);
            rear_max = rear_max.max(rear);
        }
        assert!(front_max - front_min > 0.4, "передний датчик {}", front_max - front_min);
        assert!(rear_max - rear_min < 0.15, "задний датчик {}", rear_max - rear_min);
        assert!(rear_min > 0.55);
    }

    #[test]
    fn kalina_lux_holds_the_throttle_open_at_idle() {
        let mut car = engine("B564CD04");
        drive(&mut car, 0.0, 0.0, 1.0, Gear::Neutral, 1.0);
        assert!(car.value("педаль") < 1.0);
        assert!(car.value("дроссель") > 4.0, "заслонка {}", car.value("дроссель"));
        let mut cable = engine("I201CO58");
        drive(&mut cable, 0.0, 0.0, 1.0, Gear::Neutral, 1.0);
        assert!(cable.value("дроссель") < 1.0);
    }
}
