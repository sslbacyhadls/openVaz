//! Выходы платы: PCA9685, пин разрешения импульсов, запрос кондиционера и ШИМ вентилятора.
//!
//! Выбор между этим слоем и текстовым портом делает `Drive` в `model`. Цифры калибровки
//! берутся из файла, а не из кода: крайние импульсы ещё не измерены на машине.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::model::OutputEvent;

const PCA9685_ADDR: u16 = 0x40;
const MODE1: u8 = 0x00;
const MODE2: u8 = 0x01;
const LED0_ON_L: u8 = 0x06;
const PRESCALE: u8 = 0xFE;
/// 25 МГц / (4096 × 50 Гц), округление, минус один.
const PRESCALE_50HZ: u8 = 121;
const SERVO_PERIOD_US: f32 = 20_000.0;
const FAN_PERIOD_NS: u64 = 50_000;

const I2C_SLAVE: u32 = 0x0703;
const GPIO_V2_GET_LINE: u32 = 0xC250_B407;
const GPIO_V2_LINE_SET_VALUES: u32 = 0xC010_B40B;
const GPIO_V2_LINE_FLAG_OUTPUT: u64 = 1 << 3;

/// Пин 11, PC6, gpiochip1. OE у PCA9685 активен низким уровнем.
const DEFAULT_OE_LINE: u32 = 70;
/// Пин 7, PC9, gpiochip1. Высокий уровень включает реле запроса.
const DEFAULT_AC_LINE: u32 = 73;
/// Пин 8, PH2, это PWM2. Нужен оверлей pwm, UART5 при этом занят.
const DEFAULT_PWM_CHANNEL: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServoCal {
    pub closed_us: u16,
    pub open_us: u16,
    pub invert: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardCal {
    pub servos: [Option<ServoCal>; 5],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelCmd {
    /// Импульс 50 Гц, ширина в микросекундах.
    Pulse(u16),
    /// Канал погашен, заслонку не двигаем.
    Off,
}

/// Сначала записываются импульсы, и только потом OE разрешает их.
/// Без калибровки разрешение не даётся: подтяжка держит импульсы выключенными.
pub fn servo_boot(cal: &BoardCal, targets: [f32; 5]) -> ([ChannelCmd; 5], bool) {
    let mut cmds = [ChannelCmd::Off; 5];
    let mut any = false;
    for index in 0..5 {
        if let Some(servo) = cal.servos[index] {
            cmds[index] = ChannelCmd::Pulse(pulse_us(servo, targets[index]));
            any = true;
        }
    }
    (cmds, any)
}

pub fn pulse_us(cal: ServoCal, unit: f32) -> u16 {
    let mut unit = unit.clamp(0.0, 1.0);
    if cal.invert {
        unit = 1.0 - unit;
    }
    let us = cal.closed_us as f32 + (cal.open_us as f32 - cal.closed_us as f32) * unit;
    us.round().clamp(500.0, 2500.0) as u16
}

pub fn pulse_counts(us: u16) -> u16 {
    ((us as f32 / SERVO_PERIOD_US) * 4096.0).round() as u16 & 0x0fff
}

pub fn parse_servos(text: &str) -> BoardCal {
    let mut servos = [None; 5];
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(index) = parts.next().and_then(|part| part.parse::<usize>().ok()) else {
            continue;
        };
        if index > 4 {
            continue;
        }
        let Some(closed_us) = parts.next().and_then(|part| part.parse::<u16>().ok()) else {
            continue;
        };
        let Some(open_us) = parts.next().and_then(|part| part.parse::<u16>().ok()) else {
            continue;
        };
        let invert = matches!(parts.next(), Some("1"));
        if !(500..=2500).contains(&closed_us) || !(500..=2500).contains(&open_us) {
            continue;
        }
        servos[index] = Some(ServoCal {
            closed_us,
            open_us,
            invert,
        });
    }
    BoardCal { servos }
}

pub struct Board {
    i2c: File,
    oe: File,
    ac: File,
    pwm_chip: PathBuf,
    pwm_channel: u32,
    cal: BoardCal,
    oe_enabled: bool,
}

impl Board {
    pub fn open(state_dir: Option<&Path>) -> io::Result<Self> {
        let cal_path = servo_path(state_dir);
        let text = fs::read_to_string(&cal_path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("калибровка {}: {error}", cal_path.display()),
            )
        })?;
        let cal = parse_servos(&text);
        let i2c_path = env_path("OPENVAS_I2C", "/dev/i2c-3");
        let i2c = OpenOptions::new().read(true).write(true).open(&i2c_path)?;
        unsafe {
            if libc::ioctl(i2c.as_raw_fd(), ioctl_req(I2C_SLAVE), PCA9685_ADDR as libc::c_ulong) < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        let chip = env_path("OPENVAS_GPIOCHIP", "/dev/gpiochip1");
        let oe_line = env_u32("OPENVAS_GPIO_OE", DEFAULT_OE_LINE);
        let ac_line = env_u32("OPENVAS_GPIO_AC", DEFAULT_AC_LINE);
        let oe = request_output(&chip, oe_line, "openvaz-oe", true)?;
        let ac = request_output(&chip, ac_line, "openvaz-ac", false)?;
        let pwm_chip = env_path("OPENVAS_PWM_CHIP", "/sys/class/pwm/pwmchip0");
        let pwm_channel = env_u32("OPENVAS_PWM_CHANNEL", DEFAULT_PWM_CHANNEL);
        let board = Self {
            i2c,
            oe,
            ac,
            pwm_chip,
            pwm_channel,
            cal,
            oe_enabled: false,
        };
        board.pca_init()?;
        board.set_fan(0.0)?;
        Ok(board)
    }

    pub fn command_all(&mut self, events: &[OutputEvent]) -> io::Result<()> {
        let mut targets = [0.0; 5];
        let mut fan = 0.0;
        let mut ac = false;
        for event in events {
            match event.channel {
                "0" => targets[0] = event.value,
                "1" => targets[1] = event.value,
                "2" => targets[2] = event.value,
                "3" => targets[3] = event.value,
                "4" => targets[4] = event.value,
                "вентилятор" => fan = event.value,
                "кондиционер" => ac = event.value >= 0.5,
                _ => {}
            }
        }
        let (cmds, enable) = servo_boot(&self.cal, targets);
        self.write_cmds(&cmds)?;
        if enable {
            self.set_oe(true)?;
        }
        self.set_fan(fan)?;
        self.set_ac(ac)?;
        Ok(())
    }

    pub fn command(&mut self, event: &OutputEvent) -> io::Result<()> {
        match event.channel {
            "0" | "1" | "2" | "3" | "4" => {
                let index = event.channel.as_bytes()[0] - b'0';
                let Some(cal) = self.cal.servos[index as usize] else {
                    return Ok(());
                };
                self.write_pulse(index, pulse_us(cal, event.value))?;
                if !self.oe_enabled {
                    self.set_oe(true)?;
                }
            }
            "вентилятор" => self.set_fan(event.value)?,
            "кондиционер" => self.set_ac(event.value >= 0.5)?,
            _ => {}
        }
        Ok(())
    }

    fn pca_init(&self) -> io::Result<()> {
        self.i2c_write(MODE1, &[0x10])?;
        self.i2c_write(PRESCALE, &[PRESCALE_50HZ])?;
        self.i2c_write(MODE2, &[0x04])?;
        self.i2c_write(MODE1, &[0x20])?;
        thread::sleep(Duration::from_micros(500));
        for channel in 0..5 {
            self.write_full_off(channel)?;
        }
        Ok(())
    }

    fn write_cmds(&self, cmds: &[ChannelCmd; 5]) -> io::Result<()> {
        for (channel, cmd) in cmds.iter().enumerate() {
            match *cmd {
                ChannelCmd::Pulse(us) => self.write_pulse(channel as u8, us)?,
                ChannelCmd::Off => self.write_full_off(channel as u8)?,
            }
        }
        Ok(())
    }

    fn write_pulse(&self, channel: u8, us: u16) -> io::Result<()> {
        let off = pulse_counts(us);
        let bytes = [0, 0, (off & 0xff) as u8, (off >> 8) as u8];
        self.i2c_write(LED0_ON_L + 4 * channel, &bytes)
    }

    fn write_full_off(&self, channel: u8) -> io::Result<()> {
        self.i2c_write(LED0_ON_L + 4 * channel, &[0, 0, 0, 0x10])
    }

    fn set_oe(&mut self, allow: bool) -> io::Result<()> {
        // Низкий уровень на OE разрешает импульсы.
        set_line(&self.oe, !allow)?;
        self.oe_enabled = allow;
        Ok(())
    }

    fn set_ac(&self, on: bool) -> io::Result<()> {
        set_line(&self.ac, on)
    }

    fn set_fan(&self, value: f32) -> io::Result<()> {
        let value = value.clamp(0.0, 1.0);
        let channel = self.pwm_chip.join(format!("pwm{}", self.pwm_channel));
        if !channel.exists() {
            let mut export = File::create(self.pwm_chip.join("export"))?;
            write!(export, "{}", self.pwm_channel)?;
        }
        write_sysfs(&channel.join("period"), &FAN_PERIOD_NS.to_string())?;
        let duty = (FAN_PERIOD_NS as f32 * value).round() as u64;
        write_sysfs(&channel.join("duty_cycle"), &duty.to_string())?;
        write_sysfs(&channel.join("enable"), if duty == 0 { "0" } else { "1" })?;
        Ok(())
    }

    fn i2c_write(&self, reg: u8, data: &[u8]) -> io::Result<()> {
        let mut buf = Vec::with_capacity(1 + data.len());
        buf.push(reg);
        buf.extend_from_slice(data);
        let fd = self.i2c.as_raw_fd();
        let wrote = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if wrote == buf.len() as isize {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn quiet_release(&mut self) {
        let _ = self.set_ac(false);
        let _ = self.set_fan(0.0);
        let _ = self.set_oe(false);
        for channel in 0..5 {
            let _ = self.write_full_off(channel);
        }
    }
}

impl Drop for Board {
    fn drop(&mut self) {
        self.quiet_release();
    }
}

fn servo_path(state_dir: Option<&Path>) -> PathBuf {
    state_dir
        .map(|dir| dir.join("servos"))
        .unwrap_or_else(|| PathBuf::from("servos"))
}

fn env_path(key: &str, default: &str) -> PathBuf {
    std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(default)
}

fn ioctl_req(value: u32) -> libc::Ioctl {
    value as libc::Ioctl
}

fn request_output(chip: &Path, offset: u32, consumer: &str, high: bool) -> io::Result<File> {
    let chip = OpenOptions::new().read(true).write(true).open(chip)?;
    let mut request: GpioV2LineRequest = unsafe { std::mem::zeroed() };
    request.offsets[0] = offset;
    request.num_lines = 1;
    let name = consumer.as_bytes();
    let n = name.len().min(31);
    request.consumer[..n].copy_from_slice(&name[..n]);
    request.config.flags = GPIO_V2_LINE_FLAG_OUTPUT;
    let rc = unsafe {
        libc::ioctl(
            chip.as_raw_fd(),
            ioctl_req(GPIO_V2_GET_LINE),
            &mut request,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    let line = unsafe { File::from_raw_fd(request.fd) };
    set_line(&line, high)?;
    Ok(line)
}

fn set_line(line: &File, high: bool) -> io::Result<()> {
    let values = GpioV2LineValues {
        bits: if high { 1 } else { 0 },
        mask: 1,
    };
    let rc = unsafe {
        libc::ioctl(
            line.as_raw_fd(),
            ioctl_req(GPIO_V2_LINE_SET_VALUES),
            &values as *const GpioV2LineValues,
        )
    };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn write_sysfs(path: &Path, text: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).open(path)?;
    file.write_all(text.as_bytes())
}

#[repr(C)]
struct GpioV2LineValues {
    bits: u64,
    mask: u64,
}

#[repr(C)]
struct GpioV2LineAttribute {
    id: u32,
    padding: u32,
    flags: u64,
}

#[repr(C)]
struct GpioV2LineConfigAttribute {
    attr: GpioV2LineAttribute,
    mask: u64,
}

#[repr(C)]
struct GpioV2LineConfig {
    flags: u64,
    num_attrs: u32,
    padding: [u32; 5],
    attrs: [GpioV2LineConfigAttribute; 10],
}

#[repr(C)]
struct GpioV2LineRequest {
    offsets: [u32; 64],
    consumer: [u8; 32],
    config: GpioV2LineConfig,
    num_lines: u32,
    event_buffer_size: u32,
    padding: [u32; 5],
    fd: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_follows_the_calibration_and_inversion() {
        let plain = ServoCal {
            closed_us: 1000,
            open_us: 2000,
            invert: false,
        };
        assert_eq!(pulse_us(plain, 0.0), 1000);
        assert_eq!(pulse_us(plain, 1.0), 2000);
        assert_eq!(pulse_counts(1000), 205);
        assert_eq!(pulse_counts(2000), 410);
        let flipped = ServoCal {
            invert: true,
            ..plain
        };
        assert_eq!(pulse_us(flipped, 0.0), 2000);
    }

    #[test]
    fn boot_keeps_pulses_disabled_without_calibration() {
        let (cmds, enable) = servo_boot(&BoardCal { servos: [None; 5] }, [1.0; 5]);
        assert!(!enable);
        assert!(cmds.iter().all(|cmd| *cmd == ChannelCmd::Off));
    }

    #[test]
    fn boot_writes_pulses_before_enabling_them() {
        let mut cal = BoardCal { servos: [None; 5] };
        cal.servos[1] = Some(ServoCal {
            closed_us: 1100,
            open_us: 1900,
            invert: false,
        });
        let text = "1 1100 1900 0\n# комментарий\n9 1000 2000 0\n";
        assert_eq!(parse_servos(text), cal);
        let (cmds, enable) = servo_boot(&cal, [0.0, 1.0, 0.0, 0.0, 0.0]);
        assert!(enable);
        assert_eq!(cmds[1], ChannelCmd::Pulse(1900));
        assert_eq!(cmds[0], ChannelCmd::Off);
    }

    #[test]
    fn gpio_request_matches_the_kernel_layout() {
        assert_eq!(std::mem::size_of::<GpioV2LineRequest>(), 592);
        assert_eq!(std::mem::size_of::<GpioV2LineValues>(), 16);
    }
}
