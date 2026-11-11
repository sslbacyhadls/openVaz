use std::io::{self, Write};
use std::path::PathBuf;

#[cfg(not(feature = "window"))]
use std::io::Read;
#[cfg(not(feature = "window"))]
use std::os::unix::net::UnixStream;
#[cfg(not(feature = "window"))]
use std::time::{Duration, Instant};
#[cfg(not(feature = "window"))]
use openvaz::engine::Gear;
use openvaz::firmware;
#[cfg(not(feature = "window"))]
use openvaz::link::{parse_driver_line, parse_panel_line};
#[cfg(not(feature = "window"))]
use openvaz::model::celsius_from_unit;
#[cfg(not(feature = "window"))]
use openvaz::vehicle::KeyInput;
use openvaz::vehicle::Vehicle;
#[cfg(not(feature = "window"))]
use openvaz::vehicle::DAMPERS;

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    if matches!(first.as_deref(), Some("--help") | Some("-h")) {
        print_help();
        return;
    }
    let path = first.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("emu/build/car.sock"));
    let firmware = match args.next() {
        Some(name) => match firmware::find(&name) {
            Some(firmware) => firmware,
            None => {
                eprintln!("неизвестная калибровка {name}");
                print_help();
                std::process::exit(2);
            }
        },
        None => firmware::default_firmware(),
    };
    let vehicle = Vehicle::with_firmware(firmware);
    println!(
        "{} {}  {}",
        firmware.family, firmware.id, firmware.note
    );
    #[cfg(feature = "window")]
    {
        println!("окно автомобиля, сокет {}", path.display());
        let _ = io::stdout().flush();
        openvaz::car_window::run(vehicle, path);
    }
    #[cfg(not(feature = "window"))]
    text_session(vehicle, path);
}

#[cfg(not(feature = "window"))]
fn text_session(mut vehicle: Vehicle, path: PathBuf) {
    println!("жду автомобиль на {}", path.display());
    let _ = io::stdout().flush();
    let _terminal = Terminal::take();
    loop {
        match UnixStream::connect(&path) {
            Ok(stream) => {
                println!("порт открыт");
                let _ = io::stdout().flush();
                let _ = stream.set_nonblocking(true);
                session(&mut vehicle, stream);
                println!("порт закрыт, жду снова");
                let _ = io::stdout().flush();
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn print_help() {
    println!("openvaz-car [сокет] [калибровка]");
    println!("По умолчанию сокет emu/build/car.sock и калибровка B564CD04, Калина 1 люкс.");
    println!("Клавиши, пока удерживаются: вверх или W — газ, вниз или S — тормоз, влево или A — сцепление, Enter — стартер.");
    println!("1–5 — передачи, N — нейтраль, R — задняя, I — зажигание.");
    println!("Калибровки:");
    for firmware in firmware::catalog() {
        println!("  {}  {}  {}", firmware.id, firmware.engine, firmware.note);
    }
}

#[cfg(not(feature = "window"))]
fn session(vehicle: &mut Vehicle, mut stream: UnixStream) {
    let mut incoming = Vec::new();
    let mut keys = KeyState::new();
    let mut last_tick = Instant::now();
    let mut last_status = Instant::now() - Duration::from_secs(1);
    let mut last_dash = Instant::now() - Duration::from_secs(1);
    let mut shown = String::new();
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
                    let text = text.trim();
                    if let Some(command) = parse_driver_line(text) {
                        vehicle.apply_panel(command);
                    } else if let Some((channel, value)) = parse_panel_line(text) {
                        vehicle.apply(&channel, value);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
        keys.poll();
        vehicle.set_keys(keys.input());
        let dt = last_tick.elapsed().as_secs_f32();
        if dt >= 0.05 {
            vehicle.tick(dt.min(0.5));
            last_tick = Instant::now();
        }
        if last_status.elapsed() >= Duration::from_millis(200) {
            for line in vehicle.wire_lines() {
                if writeln!(stream, "{line}").is_err() {
                    return;
                }
            }
            last_status = Instant::now();
        }
        if last_dash.elapsed() >= Duration::from_millis(200) {
            draw(vehicle, &mut shown);
            last_dash = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(not(feature = "window"))]
struct KeyState {
    gas: Instant,
    brake: Instant,
    clutch: Instant,
    crank: Instant,
    gear: Option<Gear>,
    ignition_off: bool,
    ignition_at: Instant,
    bytes: Vec<u8>,
}

#[cfg(not(feature = "window"))]
impl KeyState {
    fn new() -> Self {
        let past = Instant::now() - Duration::from_secs(1);
        Self {
            gas: past,
            brake: past,
            clutch: past,
            crank: past,
            gear: None,
            ignition_off: false,
            ignition_at: past,
            bytes: Vec::new(),
        }
    }

    fn poll(&mut self) {
        if !is_tty() {
            return;
        }
        let mut buf = [0u8; 32];
        loop {
            let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                self.bytes.extend_from_slice(&buf[..n as usize]);
            } else {
                break;
            }
        }
        let held = Instant::now() + Duration::from_millis(600);
        while !self.bytes.is_empty() {
            if self.bytes.first() == Some(&0x1b) {
                if self.bytes.len() < 3 {
                    break;
                }
                if self.bytes.get(1) == Some(&b'[') {
                    match self.bytes.get(2) {
                        Some(b'A') => self.gas = held,
                        Some(b'B') => self.brake = held,
                        Some(b'D') => self.clutch = held,
                        _ => {}
                    }
                    self.bytes.drain(..3);
                    continue;
                }
                self.bytes.remove(0);
                continue;
            }
            let byte = self.bytes.remove(0);
            match byte {
                b'w' | b'W' => self.gas = held,
                b's' | b'S' => self.brake = held,
                b'a' | b'A' => self.clutch = held,
                b'\r' | b'\n' => self.crank = held,
                b'1' => self.gear = Some(Gear::Forward(1)),
                b'2' => self.gear = Some(Gear::Forward(2)),
                b'3' => self.gear = Some(Gear::Forward(3)),
                b'4' => self.gear = Some(Gear::Forward(4)),
                b'5' => self.gear = Some(Gear::Forward(5)),
                b'n' | b'N' => self.gear = Some(Gear::Neutral),
                b'r' | b'R' => self.gear = Some(Gear::Reverse),
                b'i' | b'I' => {
                    if self.ignition_at.elapsed() > Duration::from_millis(350) {
                        self.ignition_off = !self.ignition_off;
                        self.ignition_at = Instant::now();
                    }
                }
                _ => {}
            }
        }
    }

    fn held(until: Instant) -> f32 {
        if until > Instant::now() { 1.0 } else { 0.0 }
    }

    fn input(&self) -> KeyInput {
        KeyInput {
            gas: Self::held(self.gas),
            brake: Self::held(self.brake),
            clutch: Self::held(self.clutch),
            gear: self.gear,
            ignition: Some(!self.ignition_off),
            crank: self.crank > Instant::now(),
        }
    }
}

#[cfg(not(feature = "window"))]
struct Terminal {
    saved: Option<libc::termios>,
}

#[cfg(not(feature = "window"))]
impl Terminal {
    fn take() -> Self {
        if !is_tty() {
            return Self { saved: None };
        }
        unsafe {
            let mut saved: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut saved) != 0 {
                return Self { saved: None };
            }
            let mut raw = saved;
            libc::cfmakeraw(&mut raw);
            raw.c_lflag |= libc::ISIG;
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
            let flags = libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            if let Ok(mut guard) = SAVED.lock() {
                *guard = Some(saved);
            }
            libc::signal(libc::SIGINT, on_interrupt as *const () as libc::sighandler_t);
            Self { saved: Some(saved) }
        }
    }
}

#[cfg(not(feature = "window"))]
impl Drop for Terminal {
    fn drop(&mut self) {
        restore(self.saved.as_ref());
    }
}

#[cfg(not(feature = "window"))]
static SAVED: std::sync::Mutex<Option<libc::termios>> = std::sync::Mutex::new(None);

#[cfg(not(feature = "window"))]
extern "C" fn on_interrupt(_: i32) {
    if let Ok(guard) = SAVED.lock() {
        restore(guard.as_ref());
    }
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_DFL);
        libc::raise(libc::SIGINT);
    }
}

#[cfg(not(feature = "window"))]
fn restore(saved: Option<&libc::termios>) {
    if let Some(saved) = saved {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved);
        }
    }
}

#[cfg(not(feature = "window"))]
fn draw(vehicle: &Vehicle, shown: &mut String) {
    let driver = vehicle.driver();
    if !is_tty() {
        let line = format!(
            "{}  {}  передача {}  {} об/мин  {} км/ч  газ {:.2}  тормоз {:.2}  сцепление {:.2}  охлаждение {} °C  ошибки {}",
            vehicle.firmware().id,
            vehicle.motor().label(),
            driver.gear.label(),
            vehicle.rpm().round() as i32,
            vehicle.speed_kmh().round() as i32,
            driver.gas,
            driver.brake,
            driver.clutch,
            vehicle.coolant().round() as i32,
            vehicle.error().unwrap_or("нет"),
        );
        if line != *shown {
            println!("{line}");
            let _ = io::stdout().flush();
            *shown = line;
        }
        return;
    }
    let mut frame = String::from("\x1b[H");
    frame.push_str(&fit(&format!(
        "{}  {}  мотор {}",
        vehicle.firmware().id,
        vehicle.firmware().engine,
        vehicle.motor().label()
    )));
    frame.push('\n');
    frame.push_str(&fit(&format!(
        "передача {}   обороты {:4}   скорость {:4} км/ч   дроссель {:3.0}%",
        driver.gear.label(),
        vehicle.rpm(),
        vehicle.speed_kmh(),
        vehicle.channel("дроссель")
    )));
    frame.push('\n');
    frame.push_str(&fit(&format!(
        "газ {:.2}   тормоз {:.2}   сцепление {:.2}   впрыск {:.2} мс   ДМРВ {:.1} кг/ч",
        driver.gas,
        driver.brake,
        driver.clutch,
        vehicle.channel("впрыск"),
        vehicle.channel("дмрв")
    )));
    frame.push('\n');
    frame.push_str(&fit(&format!(
        "кислород {:.2} В   коррекция {:.2}   зажигание {:.1}°   напряжение {:.1} В   РХХ {:.0}",
        vehicle.channel("кислород"),
        vehicle.channel("коррекция"),
        vehicle.channel("зажигание"),
        vehicle.channel("напряжение"),
        vehicle.channel("рхх")
    )));
    frame.push('\n');
    for index in 0..5 {
        let command = vehicle.damper_command(index);
        let mut line = format!(
            "{:<14} команда {command:.2}   положение {:.2}",
            DAMPERS[index],
            vehicle.damper_position(index)
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
        "вентилятор {:.2}   кондиционер {request}   муфта {clutch}   жидкость {} °C   ошибки {}",
        vehicle.fan_speed(),
        vehicle.coolant().round() as i32,
        vehicle.error().unwrap_or("нет")
    )));
    frame.push('\n');
    frame.push_str(&fit("клавиши: W газ  S тормоз  A сцепление  1-5 N R  I зажигание  Enter стартер"));
    frame.push('\n');
    print!("{frame}");
    let _ = io::stdout().flush();
}

#[cfg(not(feature = "window"))]
fn fit(line: &str) -> String {
    let mut text = line.to_string();
    while text.chars().count() < 88 {
        text.push(' ');
    }
    text
}

#[cfg(not(feature = "window"))]
fn is_tty() -> bool {
    unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 }
}
