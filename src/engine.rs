//! Ход Калины с ручной коробкой. Описание прошивки выбирает отсечку и холостой ход.
//! Жидкость и муфта остаются в `vehicle`: здесь только мотор, педали и скорость.

use crate::firmware::Firmware;

const WHEEL_RADIUS: f32 = 0.289;
const MASS: f32 = 1150.0;
const FINAL_DRIVE: f32 = 3.7;
/// Качение и воздух, м/с². На низших передачах тяга доходит до отсечки.
const ROLLING_ACCEL: f32 = 0.15;
const AERO_PER_SPEED2: f32 = 0.00038;
/// На сколько оборотов ниже отсечки момент ещё полный. Дальше он сходит к нулю.
const LIMIT_BAND: f32 = 250.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gear {
    Neutral,
    Forward(u8),
    Reverse,
}

impl Gear {
    pub fn ratio(self) -> f32 {
        match self {
            Gear::Neutral => 0.0,
            Gear::Forward(1) => 3.636,
            Gear::Forward(2) => 1.950,
            Gear::Forward(3) => 1.357,
            Gear::Forward(4) => 0.941,
            Gear::Forward(5) => 0.784,
            Gear::Forward(_) => 0.0,
            Gear::Reverse => 3.53,
        }
    }

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
            Gear::Forward(gear) => match gear {
                1 => "1",
                2 => "2",
                3 => "3",
                4 => "4",
                5 => "5",
                _ => "?",
            },
            Gear::Reverse => "R",
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

pub struct Engine {
    firmware: &'static Firmware,
    driver: Driver,
    motor: Motor,
    rpm: f32,
    speed_kmh: f32,
    throttle: f32,
    o2_phase: f32,
}

impl Engine {
    pub fn new(firmware: &'static Firmware) -> Self {
        Self {
            firmware,
            driver: Driver::default(),
            motor: Motor::Running,
            rpm: firmware.idle_rpm,
            speed_kmh: 0.0,
            throttle: 0.0,
            o2_phase: 0.0,
        }
    }

    pub fn firmware(&self) -> &'static Firmware {
        self.firmware
    }

    pub fn set_driver(&mut self, driver: Driver) {
        self.driver = driver;
    }

    pub fn driver(&self) -> Driver {
        self.driver
    }

    pub fn motor(&self) -> Motor {
        self.motor
    }

    pub fn rpm(&self) -> f32 {
        self.rpm
    }

    pub fn speed_kmh(&self) -> f32 {
        self.speed_kmh
    }

    pub fn throttle(&self) -> f32 {
        self.throttle
    }

    pub fn tick(&mut self, dt: f32, ac_load: bool) {
        let dt = dt.clamp(0.0, 0.5);
        let gas = if self.driver.ignition {
            self.driver.gas.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.throttle = approach(self.throttle, gas, 2.5 * dt);
        self.update_motor(dt);
        self.update_speed(dt, ac_load);
        if self.motor == Motor::Running {
            self.o2_phase += dt * (3.0 + self.rpm / 500.0);
        }
    }

    pub fn value(&self, key: &str, coolant: f32, radiator_fan: bool, ac_load: bool) -> f32 {
        match key {
            "обороты" => self.rpm,
            "обороты_хх" => self.firmware.idle_rpm + if ac_load { 100.0 } else { 0.0 },
            "скорость" => self.speed_kmh.abs(),
            "дроссель" => self.throttle * 100.0,
            "воздух" => 25.0,
            "дмрв" => self.maf_kg_h(),
            "впрыск" => self.injection_ms(),
            "зажигание" => self.advance(),
            "напряжение" => self.battery(radiator_fan),
            "коррекция" => 1.0,
            "кислород" => self.oxygen(),
            "кислород2" => self.rear_oxygen(coolant),
            "рхх" => self.iac(coolant, ac_load),
            "продувка" => {
                if self.motor == Motor::Running && self.throttle > 0.2 {
                    15.0 + self.throttle * 20.0
                } else {
                    0.0
                }
            }
            "детонация" => {
                if self.motor == Motor::Running && self.throttle > 0.72 && self.rpm > 3200.0 {
                    1.5
                } else {
                    0.0
                }
            }
            "вентилятор_охлаждения" if radiator_fan => 1.0,
            "вентилятор_охлаждения" => 0.0,
            _ => 0.0,
        }
    }

    fn update_motor(&mut self, dt: f32) {
        if !self.driver.ignition {
            self.motor = Motor::Off;
        } else if self.driver.crank && self.motor != Motor::Running {
            self.motor = Motor::Cranking;
        }
        match self.motor {
            Motor::Off | Motor::Stalled => {
                self.rpm = approach(self.rpm, 0.0, 1800.0 * dt);
            }
            Motor::Cranking => {
                self.rpm = approach(self.rpm, 480.0, 900.0 * dt);
                if self.rpm > 380.0 && self.driver.ignition {
                    self.motor = Motor::Running;
                }
            }
            Motor::Running => {
                let free = self.free_rpm();
                let ratio = self.driver.gear.ratio();
                let locked = clutch_lock(self.driver.clutch);
                let road = rpm_from_speed(self.speed_kmh, ratio);
                // Пока колёса медленнее холостого хода, а педаль газа нажата, сцепление буксует:
                // мотор остаётся на оборотах трогания, а не падает в ноль вместе с коробкой.
                let launch = free.clamp(self.firmware.idle_rpm, 1800.0);
                let coupled = if self.driver.gas > 0.15 {
                    road.max(launch)
                } else {
                    road
                }
                .min(self.firmware.rev_limit);
                let target = if ratio == 0.0 {
                    free
                } else {
                    free * (1.0 - locked) + coupled * locked
                };
                self.rpm = approach(self.rpm, target.min(self.firmware.rev_limit), 12_000.0 * dt);
                let dropped = ratio != 0.0
                    && self.driver.clutch < 0.15
                    && self.speed_kmh.abs() < 2.0
                    && self.rpm < 520.0
                    && self.driver.gas < 0.2
                    && self.throttle < 0.35;
                if dropped {
                    self.motor = Motor::Stalled;
                }
            }
        }
    }

    fn update_speed(&mut self, dt: f32, ac_load: bool) {
        let ratio = self.driver.gear.ratio();
        let locked = clutch_lock(self.driver.clutch);
        let mut accel = 0.0;
        let sign = if matches!(self.driver.gear, Gear::Reverse) {
            -1.0
        } else {
            1.0
        };
        if self.motor == Motor::Running && ratio != 0.0 && locked > 0.05 {
            let road = rpm_from_speed(self.speed_kmh, ratio);
            let headroom = ((self.firmware.rev_limit - road) / LIMIT_BAND).clamp(0.0, 1.0);
            if self.throttle >= 0.02 {
                let mut torque = self.drive_torque() * locked * headroom;
                if ac_load {
                    torque = (torque - 8.0).max(0.0);
                }
                accel += sign * wheel_accel(torque, ratio);
            }
            if self.throttle < 0.12 {
                let braking = (1.0 - self.throttle / 0.12) * 25.0 * locked;
                accel -= sign * wheel_accel(braking, ratio);
            }
        }
        let along = travel_sign(self.speed_kmh);
        if along != 0.0 {
            let speed_ms = self.speed_kmh / 3.6;
            accel -= ROLLING_ACCEL * along + AERO_PER_SPEED2 * speed_ms.abs() * speed_ms;
        }
        accel -= self.driver.brake.clamp(0.0, 1.0) * 7.0 * along;
        self.speed_kmh += accel * dt * 3.6;
        if self.driver.brake > 0.2 && self.speed_kmh.abs() < 0.4 {
            self.speed_kmh = 0.0;
        }
    }

    fn drive_torque(&self) -> f32 {
        let kilo = (self.rpm.max(0.0) / 1000.0).min(6.5);
        let shape = if kilo < 1.2 {
            0.35 + 0.45 * (kilo / 1.2)
        } else {
            (1.0 - (kilo - 3.5).abs() / 5.5).clamp(0.62, 1.0)
        };
        let load = (self.throttle * 0.92 + 0.08).min(1.0);
        self.firmware.peak_nm * shape * load
    }

    fn free_rpm(&self) -> f32 {
        let rise = self.firmware.rev_limit - self.firmware.idle_rpm;
        self.firmware.idle_rpm + self.throttle * rise
    }

    fn maf_kg_h(&self) -> f32 {
        if self.motor != Motor::Running {
            return 0.0;
        }
        (self.rpm / 1000.0) * (4.0 + self.throttle * 28.0)
    }

    fn injection_ms(&self) -> f32 {
        if self.motor != Motor::Running {
            return 0.0;
        }
        2.2 + self.throttle * 8.0 + (self.rpm / self.firmware.rev_limit) * 2.0
    }

    fn advance(&self) -> f32 {
        if self.motor != Motor::Running {
            return 0.0;
        }
        (8.0 + self.rpm / 1000.0 * 3.0 - self.throttle * 6.0).clamp(4.0, 36.0)
    }

    fn battery(&self, radiator_fan: bool) -> f32 {
        match self.motor {
            Motor::Cranking => 10.4,
            Motor::Running => {
                if radiator_fan {
                    13.6
                } else {
                    14.2
                }
            }
            Motor::Off | Motor::Stalled => 12.4,
        }
    }

    fn oxygen(&self) -> f32 {
        if self.motor != Motor::Running {
            return 0.45;
        }
        0.45 + self.o2_phase.sin() * 0.38
    }

    fn rear_oxygen(&self, coolant: f32) -> f32 {
        if self.motor == Motor::Running && coolant > 60.0 {
            0.68
        } else {
            0.45
        }
    }

    fn iac(&self, coolant: f32, ac_load: bool) -> f32 {
        if self.motor != Motor::Running {
            return 130.0;
        }
        let cold = (70.0 - coolant).max(0.0) * 1.4;
        let ac = if ac_load { 28.0 } else { 0.0 };
        36.0 + cold + ac
    }
}

fn clutch_lock(pedal: f32) -> f32 {
    (1.0 - pedal.clamp(0.0, 1.0)).powf(1.4)
}

fn wheel_accel(torque_nm: f32, ratio: f32) -> f32 {
    torque_nm * ratio * FINAL_DRIVE / WHEEL_RADIUS / MASS
}

fn travel_sign(speed_kmh: f32) -> f32 {
    if speed_kmh.abs() < 0.05 {
        0.0
    } else {
        speed_kmh.signum()
    }
}

fn rpm_from_speed(speed_kmh: f32, ratio: f32) -> f32 {
    let wheel_rpm = speed_kmh.abs() / 3.6 / (2.0 * std::f32::consts::PI * WHEEL_RADIUS) * 60.0;
    wheel_rpm * ratio * FINAL_DRIVE
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
            engine.tick(0.05, false);
            left -= 0.05;
        }
    }

    #[test]
    fn neutral_gas_reaches_the_firmware_limit() {
        let mut motor = engine("январь-7.2");
        drive(&mut motor, 1.0, 0.0, 1.0, Gear::Neutral, 8.0);
        let limit = motor.firmware().rev_limit;
        assert!(motor.rpm() > limit - 250.0, "обороты {}", motor.rpm());
        assert!(motor.speed_kmh().abs() < 1.0);
    }

    #[test]
    fn first_gear_picks_up_speed_and_brake_stops_it() {
        let mut motor = engine("итэлма-м74");
        drive(&mut motor, 0.7, 0.0, 0.35, Gear::Forward(1), 8.0);
        assert!(motor.speed_kmh() > 15.0, "скорость {}", motor.speed_kmh());
        drive(&mut motor, 0.0, 1.0, 1.0, Gear::Forward(1), 4.0);
        assert!(
            motor.speed_kmh().abs() < 1.0,
            "после тормоза {}",
            motor.speed_kmh()
        );
    }

    #[test]
    fn second_gear_pulls_after_the_first_gear_limit() {
        let mut motor = engine("итэлма-м74");
        drive(&mut motor, 1.0, 0.0, 0.0, Gear::Forward(1), 12.0);
        let limit = motor.firmware().rev_limit;
        assert!(
            motor.rpm() > limit - 200.0,
            "первая передача {}",
            motor.rpm()
        );
        let speed_at_limit = motor.speed_kmh();
        drive(&mut motor, 1.0, 0.0, 0.0, Gear::Forward(2), 0.5);
        assert!(
            motor.rpm() < limit - 800.0,
            "после переключения {}",
            motor.rpm()
        );
        assert!(motor.speed_kmh() > speed_at_limit * 0.9);
        drive(&mut motor, 1.0, 0.0, 0.0, Gear::Forward(2), 12.0);
        assert!(
            motor.rpm() > limit - 200.0,
            "вторая передача {}",
            motor.rpm()
        );
        assert!(
            motor.speed_kmh() > speed_at_limit + 20.0,
            "скорость {}",
            motor.speed_kmh()
        );
    }

    #[test]
    fn dropped_clutch_at_rest_stalls() {
        let mut motor = engine("итэлма-м74");
        drive(&mut motor, 0.0, 0.0, 0.0, Gear::Forward(1), 2.0);
        assert_eq!(motor.motor(), Motor::Stalled);
    }

    #[test]
    fn starter_restarts_a_stalled_engine() {
        let mut motor = engine("итэлма-м74");
        drive(&mut motor, 0.0, 0.0, 0.0, Gear::Forward(1), 2.0);
        motor.set_driver(Driver {
            gear: Gear::Neutral,
            clutch: 1.0,
            ignition: true,
            crank: true,
            ..Driver::default()
        });
        motor.tick(0.5, false);
        motor.tick(0.5, false);
        assert_eq!(motor.motor(), Motor::Running);
    }
}
