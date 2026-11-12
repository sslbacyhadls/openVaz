use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Сколько тишины на порту, после которой панель снова пишет «нет связи».
pub const LINK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq)]
pub enum CarMessage {
    Coolant(i32),
    Errors(Vec<String>),
    Clutch(bool),
    Firmware(String),
    Calibration(String),
    Reading(String, f32),
    Cabin(f32),
    Motor(String),
    Gear(String),
    Pedals(f32, f32, f32),
    /// Положение заслонки 0–4 в долях хода. `None` — датчик вне диапазона, заслонку не трогать.
    Position(String, Option<f32>),
}

pub fn car_path() -> PathBuf {
    std::env::var_os("OPENVAZ_CAR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/ttyAMA1"))
}

pub struct CarPort {
    file: std::fs::File,
    incoming: Vec<u8>,
}

impl CarPort {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(path)?;
        let fd = file.as_raw_fd();
        unsafe {
            let mut cfg: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut cfg) == 0 {
                libc::cfmakeraw(&mut cfg);
                libc::cfsetispeed(&mut cfg, libc::B115200);
                libc::cfsetospeed(&mut cfg, libc::B115200);
                cfg.c_cflag |= libc::CLOCAL | libc::CREAD;
                libc::tcsetattr(fd, libc::TCSANOW, &cfg);
            }
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
        Ok(Self {
            file,
            incoming: Vec::new(),
        })
    }

    pub fn fd(&self) -> i32 {
        self.file.as_raw_fd()
    }

    pub fn send_command(&mut self, channel: &str, value: f32) -> io::Result<()> {
        let line = format!("выход {channel} значение {value:.2}\n");
        match self.file.write_all(line.as_bytes()) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            other => other,
        }
    }

    pub fn poll_messages(&mut self) -> io::Result<Vec<CarMessage>> {
        let mut buf = [0u8; 512];
        loop {
            match self.file.read(&mut buf) {
                Ok(0) => {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "порт закрыт"));
                }
                Ok(n) => {
                    self.incoming.extend_from_slice(&buf[..n]);
                    if self.incoming.len() > 8192 {
                        self.incoming.clear();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        let mut messages = Vec::new();
        while let Some(pos) = self.incoming.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.incoming.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line);
            if let Some(message) = parse_car_line(text.trim()) {
                messages.push(message);
            }
        }
        Ok(messages)
    }
}

pub fn parse_car_line(line: &str) -> Option<CarMessage> {
    let mut parts = line.split_whitespace();
    match parts.next()? {
        "охлаждение" => {
            let value = parts.next()?.parse().ok()?;
            Some(CarMessage::Coolant(value))
        }
        "ошибки" => {
            let rest: Vec<String> = parts.map(str::to_string).collect();
            if rest.is_empty() || rest == ["нет"] {
                Some(CarMessage::Errors(Vec::new()))
            } else {
                Some(CarMessage::Errors(rest))
            }
        }
        "муфта" => match parts.next()? {
            "1" => Some(CarMessage::Clutch(true)),
            "0" => Some(CarMessage::Clutch(false)),
            _ => None,
        },
        "прошивка" => {
            let id = parts.next()?.to_string();
            if id.is_empty() {
                None
            } else {
                Some(CarMessage::Firmware(id))
            }
        }
        "калибровка" => {
            let text = parts.collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                None
            } else {
                Some(CarMessage::Calibration(text))
            }
        }
        "показание" => {
            let key = parts.next()?.to_string();
            let value = parts.next()?.parse::<f32>().ok()?;
            if key.is_empty() || !value.is_finite() {
                None
            } else {
                Some(CarMessage::Reading(key, value))
            }
        }
        "салон" => {
            let value = parts.next()?.parse::<f32>().ok()?;
            if value.is_finite() {
                Some(CarMessage::Cabin(value))
            } else {
                None
            }
        }
        "мотор" => {
            let state = parts.next()?.to_string();
            if state.is_empty() {
                None
            } else {
                Some(CarMessage::Motor(state))
            }
        }
        "передача" => {
            let gear = parts.next()?.to_string();
            if gear.is_empty() {
                None
            } else {
                Some(CarMessage::Gear(gear))
            }
        }
        "педали" => {
            let gas = parts.next()?.parse::<f32>().ok()?;
            let brake = parts.next()?.parse::<f32>().ok()?;
            let clutch = parts.next()?.parse::<f32>().ok()?;
            if gas.is_finite() && brake.is_finite() && clutch.is_finite() {
                Some(CarMessage::Pedals(gas, brake, clutch))
            } else {
                None
            }
        }
        "положение" => {
            let channel = parts.next()?.to_string();
            if channel.is_empty() {
                return None;
            }
            match parts.next()? {
                "нет" => Some(CarMessage::Position(channel, None)),
                text => {
                    let value = text.parse::<f32>().ok()?;
                    if !value.is_finite() {
                        return None;
                    }
                    Some(CarMessage::Position(channel, Some(value)))
                }
            }
        }
        _ => None,
    }
}

pub fn parse_panel_line(line: &str) -> Option<(String, f32)> {
    let rest = line.trim().strip_prefix("выход ")?;
    let (channel, value) = rest.rsplit_once(" значение ")?;
    if channel.is_empty() {
        return None;
    }
    let value = value.trim().parse::<f32>().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some((channel.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_and_car_lines_round_trip() {
        let (channel, value) = parse_panel_line("выход вентилятор значение 0.25\n").unwrap();
        assert_eq!(channel, "вентилятор");
        assert!((value - 0.25).abs() < 0.001);
        assert!(parse_panel_line("старт").is_none());
        assert_eq!(
            parse_car_line("охлаждение 90"),
            Some(CarMessage::Coolant(90))
        );
        assert_eq!(
            parse_car_line("ошибки нет"),
            Some(CarMessage::Errors(vec![]))
        );
        assert_eq!(
            parse_car_line("ошибки P0217"),
            Some(CarMessage::Errors(vec!["P0217".into()]))
        );
        assert_eq!(parse_car_line("муфта 1"), Some(CarMessage::Clutch(true)));
        assert_eq!(parse_car_line("муфта 0"), Some(CarMessage::Clutch(false)));
        assert!(parse_car_line("муфта да").is_none());
        assert_eq!(
            parse_car_line("прошивка итэлма-м74"),
            Some(CarMessage::Firmware("итэлма-м74".into()))
        );
        assert_eq!(
            parse_car_line("калибровка I444CE08"),
            Some(CarMessage::Calibration("I444CE08".into()))
        );
        assert_eq!(
            parse_car_line("показание обороты 860"),
            Some(CarMessage::Reading("обороты".into(), 860.0))
        );
        assert!(parse_car_line("показание обороты нет").is_none());
        assert_eq!(parse_car_line("салон 21.5"), Some(CarMessage::Cabin(21.5)));
        assert_eq!(
            parse_car_line("педали 0.20 1.00 0.00"),
            Some(CarMessage::Pedals(0.2, 1.0, 0.0))
        );
        assert_eq!(
            parse_car_line("передача 1"),
            Some(CarMessage::Gear("1".into()))
        );
        assert_eq!(
            parse_car_line("положение 1 0.40"),
            Some(CarMessage::Position("1".into(), Some(0.40)))
        );
        assert_eq!(
            parse_car_line("положение 2 нет"),
            Some(CarMessage::Position("2".into(), None))
        );
    }
}
