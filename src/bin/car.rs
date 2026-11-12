use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use openvaz::engine::{Driver, Gear, Motor};
use openvaz::firmware::{self, Firmware};
use openvaz::link::parse_panel_line;
use openvaz::model::celsius_from_unit;
use openvaz::vehicle::{Vehicle, DAMPERS};

const HOLD: Duration = Duration::from_millis(400);
const IGNITION_GAP: Duration = Duration::from_millis(350);

fn main() {
    let (path, chosen) = match launch(std::env::args().skip(1)) {
        Launch::Help => {
            print_help();
            return;
        }
        Launch::Ready(path, firmware) => (path, firmware),
        Launch::Unknown(name) => {
            eprintln!("неизвестная прошивка: {name}");
            for firmware in firmware::catalog() {
                eprintln!("{}", firmware.id);
            }
            std::process::exit(2);
        }
    };
    let mut vehicle = Vehicle::with_firmware(chosen);
    println!("жду автомобиль на {}", path.display());
    println!("прошивка {}  {}", chosen.id, chosen.calibration);
    let _ = io::stdout().flush();
    let terminal = RawTerm::claim();
    loop {
        match UnixStream::connect(&path) {
            Ok(stream) => {
                println!("порт открыт");
                let _ = io::stdout().flush();
                let _ = stream.set_nonblocking(true);
                session(&mut vehicle, stream, terminal.is_some());
                println!("порт закрыт, жду снова");
                let _ = io::stdout().flush();
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

enum Launch {
    Help,
    Ready(PathBuf, &'static Firmware),
    Unknown(String),
}

fn launch<I>(args: I) -> Launch
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    let default_socket = PathBuf::from("emu/build/car.sock");
    match args.as_slice() {
        [] => Launch::Ready(default_socket, firmware::default_firmware()),
        [flag] if flag == "--help" || flag == "-h" => Launch::Help,
        [only] => match firmware::find(only) {
            Some(firmware) => Launch::Ready(default_socket, firmware),
            None => Launch::Ready(PathBuf::from(only), firmware::default_firmware()),
        },
        [socket, name] => match firmware::find(name) {
            Some(firmware) => Launch::Ready(PathBuf::from(socket), firmware),
            None => Launch::Unknown(name.clone()),
        },
        _ => Launch::Unknown(args[1].clone()),
    }
}

fn print_help() {
    println!("openvaz-car [сокет] [прошивка]");
    println!("Слушает команды панели и отвечает климатом, жидкостью и показаниями ЭБУ.");
    println!("По умолчанию сокет emu/build/car.sock и прошивка итэлма-м74.");
    println!("Прошивки:");
    for firmware in firmware::catalog() {
        println!("  {}  {}  {}", firmware.id, firmware.family, firmware.calibration);
    }
}

struct Keys {
    gas_until: Option<Instant>,
    brake_until: Option<Instant>,
    clutch_until: Option<Instant>,
    clutch: f32,
    crank_until: Option<Instant>,
    ignition: bool,
    ignition_at: Instant,
    gear: Gear,
    pending: Vec<u8>,
}

impl Keys {
    fn new() -> Self {
        Self {
            gas_until: None,
            brake_until: None,
            clutch_until: None,
            clutch: 0.0,
            crank_until: None,
            ignition: true,
            ignition_at: Instant::now() - IGNITION_GAP,
            gear: Gear::Neutral,
            pending: Vec::new(),
        }
    }

    fn driver(&self) -> Driver {
        Driver {
            gas: held(self.gas_until),
            brake: held(self.brake_until),
            clutch: self.clutch,
            gear: self.gear,
            ignition: self.ignition,
            crank: held(self.crank_until) > 0.0,
        }
    }

    fn follow(&mut self, dt: f32) {
        let target = held(self.clutch_until);
        let step = dt.clamp(0.0, 0.5) * 0.8;
        let delta = target - self.clutch;
        if delta.abs() <= step {
            self.clutch = target;
        } else {
            self.clutch += step.copysign(delta);
        }
    }

    fn poll(&mut self) {
        let mut buf = [0u8; 64];
        loop {
            let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            self.pending.extend_from_slice(&buf[..n as usize]);
        }
        self.drain();
    }

    fn drain(&mut self) {
        while !self.pending.is_empty() {
            if self.pending[0] == 0x1b {
                if self.pending.len() < 3 {
                    break;
                }
                if self.pending[1] == b'[' {
                    match self.pending[2] {
                        b'A' => self.hold_gas(),
                        b'B' => self.hold_brake(),
                        b'D' => self.hold_clutch(),
                        _ => {}
                    }
                }
                self.pending.drain(..3);
                continue;
            }
            let width = utf8_len(self.pending[0]);
            if self.pending.len() < width {
                break;
            }
            let text = String::from_utf8_lossy(&self.pending[..width]).into_owned();
            self.pending.drain(..width);
            self.on_text(&text);
        }
    }

    fn hold_gas(&mut self) {
        self.gas_until = Some(Instant::now() + HOLD);
    }

    fn hold_brake(&mut self) {
        self.brake_until = Some(Instant::now() + HOLD);
    }

    fn hold_clutch(&mut self) {
        self.clutch_until = Some(Instant::now() + HOLD);
    }

    fn on_text(&mut self, text: &str) {
        match text {
            "w" | "W" | "ц" | "Ц" => self.hold_gas(),
            "s" | "S" | "ы" | "Ы" => self.hold_brake(),
            "a" | "A" | "ф" | "Ф" => self.hold_clutch(),
            "1" => self.gear = Gear::Forward(1),
            "2" => self.gear = Gear::Forward(2),
            "3" => self.gear = Gear::Forward(3),
            "4" => self.gear = Gear::Forward(4),
            "5" => self.gear = Gear::Forward(5),
            "n" | "N" | "т" | "Т" => self.gear = Gear::Neutral,
            "r" | "R" | "к" | "К" => self.gear = Gear::Reverse,
            "i" | "I" | "ш" | "Ш" => self.toggle_ignition(),
            "\n" | "\r" => self.crank_until = Some(Instant::now() + HOLD),
            _ => {}
        }
    }

    fn toggle_ignition(&mut self) {
        if self.ignition_at.elapsed() < IGNITION_GAP {
            return;
        }
        self.ignition = !self.ignition;
        self.ignition_at = Instant::now();
    }
}

fn held(until: Option<Instant>) -> f32 {
    if until.is_some_and(|moment| Instant::now() < moment) {
        1.0
    } else {
        0.0
    }
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first < 0xE0 {
        2
    } else if first < 0xF0 {
        3
    } else {
        4
    }
}

fn session(vehicle: &mut Vehicle, mut stream: UnixStream, keyboard: bool) {
    let mut incoming = Vec::new();
    let mut last_tick = Instant::now();
    let mut last_status = Instant::now() - Duration::from_secs(1);
    let mut last_dash = Instant::now() - Duration::from_secs(1);
    let mut shown = String::new();
    let mut keys = Keys::new();
    loop {
        let mut buf = [0u8; 512];
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                incoming.extend_from_slice(&buf[..n]);
                if incoming.len() > 8192 {
                    incoming.clear();
                }
                while let Some(pos) = incoming.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = incoming.drain(..=pos).collect();
                    let text = String::from_utf8_lossy(&line);
                    if let Some((channel, value)) = parse_panel_line(&text) {
                        vehicle.apply(&channel, value);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
        if keyboard {
            keys.poll();
        }
        let dt = last_tick.elapsed().as_secs_f32();
        if dt >= 0.05 {
            keys.follow(dt);
            vehicle.set_driver(keys.driver());
            vehicle.tick(dt.min(0.5));
            last_tick = Instant::now();
        }
        if last_status.elapsed() >= Duration::from_millis(200) {
            for line in vehicle.status_lines() {
                if write!(stream, "{line}\n").is_err() {
                    return;
                }
            }
            for line in vehicle.wire_lines() {
                if write!(stream, "{line}\n").is_err() {
                    return;
                }
            }
            last_status = Instant::now();
        }
        if last_dash.elapsed() >= Duration::from_millis(200) {
            draw(vehicle, &mut shown, keyboard);
            last_dash = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn draw(vehicle: &Vehicle, shown: &mut String, keyboard: bool) {
    if !is_tty() {
        let line = compact(vehicle);
        if line != *shown {
            println!("{line}");
            let _ = io::stdout().flush();
            *shown = line;
        }
        return;
    }
    let engine = vehicle.engine();
    let driver = engine.driver();
    let mut frame = String::from("\x1b[H");
    frame.push_str(&fit(&format!(
        "Автомобиль  {}  {}",
        vehicle.firmware().family,
        vehicle.firmware().calibration
    )));
    frame.push('\n');
    frame.push_str(&fit(&format!(
        "мотор {:<10} передача {}   обороты {:4.0}   скорость {:3.0} км/ч",
        motor_label(engine.motor()),
        driver.gear.label(),
        engine.rpm(),
        engine.speed_kmh()
    )));
    frame.push('\n');
    frame.push_str(&fit(&format!(
        "газ {:.2}   тормоз {:.2}   сцепление {:.2}   зажигание {}",
        driver.gas,
        driver.brake,
        driver.clutch,
        if driver.ignition { "вкл" } else { "выкл" }
    )));
    frame.push('\n');
    for index in 0..5 {
        let command = vehicle.damper_command(index);
        let position = vehicle.damper_position(index);
        let mut line = format!(
            "{:<14} команда {command:.2}   положение {position:.2}",
            DAMPERS[index]
        );
        if index == 1 {
            line.push_str(&format!("   задание {} °C", celsius_from_unit(command)));
        }
        frame.push_str(&fit(&line));
        frame.push('\n');
    }
    let request = if vehicle.ac_request() { "вкл" } else { "выкл" };
    let clutch = if vehicle.clutch { "вкл" } else { "выкл" };
    frame.push_str(&fit(&format!(
        "вентилятор     команда {:.2}   факт {:.2}",
        vehicle.fan_command(),
        vehicle.fan_speed()
    )));
    frame.push('\n');
    frame.push_str(&fit(&format!(
        "кондиционер    запрос {request}    муфта {clutch}"
    )));
    frame.push('\n');
    frame.push_str(&fit(&format!(
        "охлаждение     {} °C",
        vehicle.coolant.round() as i32
    )));
    frame.push('\n');
    frame.push_str(&fit(&format!(
        "ошибки         {}",
        vehicle.error.unwrap_or("нет")
    )));
    frame.push('\n');
    if keyboard {
        frame.push_str(&fit(
            "газ W/↑  тормоз S/↓  сцепление A/←  передачи 1-5 N R  зажигание I  стартёр Enter",
        ));
        frame.push('\n');
    }
    print!("{frame}");
    let _ = io::stdout().flush();
}

fn motor_label(motor: Motor) -> &'static str {
    match motor {
        Motor::Off => "выкл",
        Motor::Cranking => "стартер",
        Motor::Running => "работает",
        Motor::Stalled => "заглох",
    }
}

fn compact(vehicle: &Vehicle) -> String {
    let engine = vehicle.engine();
    let clutch = if vehicle.clutch { "вкл" } else { "выкл" };
    let request = if vehicle.ac_request() { "вкл" } else { "выкл" };
    format!(
        "{}  {} {} об/мин  {} км/ч  передача {}  охлаждение {} °C  задание {} °C  заслонка {:.2}  муфта {clutch}  запрос {request}  вентилятор {:.2}  ошибки {}",
        vehicle.firmware().id,
        motor_label(engine.motor()),
        engine.rpm().round() as i32,
        engine.speed_kmh().round() as i32,
        engine.driver().gear.label(),
        vehicle.coolant.round() as i32,
        celsius_from_unit(vehicle.damper_command(1)),
        vehicle.damper_position(1),
        vehicle.fan_speed(),
        vehicle.error.unwrap_or("нет")
    )
}

fn fit(line: &str) -> String {
    let mut text = line.to_string();
    while text.chars().count() < 78 {
        text.push(' ');
    }
    text
}

fn is_tty() -> bool {
    unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 }
}

struct RawTerm {
    original: libc::termios,
}

impl RawTerm {
    fn claim() -> Option<Self> {
        if unsafe { libc::isatty(libc::STDIN_FILENO) } != 1 {
            return None;
        }
        unsafe {
            let mut original = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut original) != 0 {
                return None;
            }
            let mut raw = original;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO);
            raw.c_cc[libc::VMIN] = 0;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) != 0 {
                return None;
            }
            let flags = libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            print!("\x1b[?25l\x1b[2J\x1b[H");
            let _ = io::stdout().flush();
            Some(Self { original })
        }
    }
}

impl Drop for RawTerm {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
        }
        print!("\x1b[?25h\n");
        let _ = io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_name_is_the_factory_firmware() {
        match launch(std::iter::empty()) {
            Launch::Ready(_, firmware) => assert_eq!(firmware.id, "итэлма-м74"),
            _ => panic!("ожидалась заводская прошивка"),
        }
    }

    #[test]
    fn a_firmware_name_selects_it_and_keeps_the_socket() {
        match launch(["январь-7.2".into()]) {
            Launch::Ready(path, firmware) => {
                assert_eq!(firmware.id, "январь-7.2");
                assert_eq!(path, PathBuf::from("emu/build/car.sock"));
            }
            _ => panic!("ожидалась прошивка"),
        }
    }

    #[test]
    fn unknown_name_is_rejected() {
        match launch(["emu/build/car.sock".into(), "нет-такой".into()]) {
            Launch::Unknown(name) => assert_eq!(name, "нет-такой"),
            _ => panic!("ожидался отказ"),
        }
    }

    #[test]
    fn a_socket_alone_keeps_the_factory_firmware() {
        match launch(["/tmp/car.sock".into()]) {
            Launch::Ready(path, firmware) => {
                assert_eq!(path, PathBuf::from("/tmp/car.sock"));
                assert_eq!(firmware.id, "итэлма-м74");
            }
            _ => panic!("ожидался сокет"),
        }
    }
}
