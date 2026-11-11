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
    Family(String),
    Calibration(String),
    Reading(String, f32),
    Motion(String, f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DriverCommand {
    Gas(f32),
    Brake(f32),
    Clutch(f32),
    Gear(i32),
    Ignition(bool),
    Crank(bool),
}

pub fn car_path() -> PathBuf {
    std::env::var_os("OPENVAS_CAR")
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
        self.send_line(&format!("выход {channel} значение {value:.2}"))
    }

    pub fn send_line(&mut self, line: &str) -> io::Result<()> {
        let mut text = line.to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        match self.file.write_all(text.as_bytes()) {
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
            let name = parts.collect::<Vec<_>>().join(" ");
            if name.is_empty() {
                None
            } else {
                Some(CarMessage::Family(name))
            }
        }
        "калибровка" => Some(CarMessage::Calibration(parts.next()?.to_string())),
        "показание" | "ход" => {
            let motion = line.starts_with("ход");
            let key = parts.next()?.to_string();
            let value = parts.next()?.parse().ok()?;
            if motion {
                Some(CarMessage::Motion(key, value))
            } else {
                Some(CarMessage::Reading(key, value))
            }
        }
        _ => None,
    }
}

pub fn parse_driver_line(line: &str) -> Option<DriverCommand> {
    let rest = line.trim().strip_prefix("водитель ")?;
    let mut parts = rest.split_whitespace();
    let value = |token: Option<&str>| token?.parse::<f32>().ok();
    match parts.next()? {
        "газ" => Some(DriverCommand::Gas(value(parts.next())?.clamp(0.0, 1.0))),
        "тормоз" => Some(DriverCommand::Brake(value(parts.next())?.clamp(0.0, 1.0))),
        "сцепление" => Some(DriverCommand::Clutch(value(parts.next())?.clamp(0.0, 1.0))),
        "передача" => Some(DriverCommand::Gear(parts.next()?.parse().ok()?)),
        "зажигание" => Some(DriverCommand::Ignition(parts.next()? != "0")),
        "стартер" => Some(DriverCommand::Crank(parts.next()? != "0")),
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
        assert_eq!(parse_car_line("охлаждение 90"), Some(CarMessage::Coolant(90)));
        assert_eq!(parse_car_line("ошибки нет"), Some(CarMessage::Errors(vec![])));
        assert_eq!(
            parse_car_line("ошибки P0217"),
            Some(CarMessage::Errors(vec!["P0217".into()]))
        );
        assert_eq!(parse_car_line("муфта 1"), Some(CarMessage::Clutch(true)));
        assert_eq!(parse_car_line("муфта 0"), Some(CarMessage::Clutch(false)));
        assert!(parse_car_line("муфта да").is_none());
        assert_eq!(
            parse_car_line("прошивка Январь 7.2"),
            Some(CarMessage::Family("Январь 7.2".into()))
        );
        assert_eq!(
            parse_car_line("калибровка I201CO58"),
            Some(CarMessage::Calibration("I201CO58".into()))
        );
        assert_eq!(
            parse_car_line("показание обороты 860"),
            Some(CarMessage::Reading("обороты".into(), 860.0))
        );
        assert_eq!(parse_driver_line("водитель газ 0.5"), Some(DriverCommand::Gas(0.5)));
        assert_eq!(parse_driver_line("водитель передача -1"), Some(DriverCommand::Gear(-1)));
    }
}
