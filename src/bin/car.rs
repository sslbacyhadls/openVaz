use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use openvaz::link::parse_panel_line;
use openvaz::model::celsius_from_unit;
use openvaz::vehicle::{Vehicle, DAMPERS};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = match args.next().as_deref() {
        Some("--help") | Some("-h") => {
            println!("openvaz-car [сокет]");
            println!("Слушает команды панели и отвечает температурой, ошибками и муфтой.");
            println!("По умолчанию сокет emu/build/car.sock");
            return;
        }
        Some(path) => PathBuf::from(path),
        None => PathBuf::from("emu/build/car.sock"),
    };
    let mut vehicle = Vehicle::new();
    println!("жду автомобиль на {}", path.display());
    let _ = io::stdout().flush();
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

fn session(vehicle: &mut Vehicle, mut stream: UnixStream) {
    let mut incoming = Vec::new();
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
                    if let Some((channel, value)) = parse_panel_line(&text) {
                        vehicle.apply(&channel, value);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
        let dt = last_tick.elapsed().as_secs_f32();
        if dt >= 0.05 {
            vehicle.tick(dt.min(0.5));
            last_tick = Instant::now();
        }
        if last_status.elapsed() >= Duration::from_millis(200) {
            for line in vehicle.status_lines() {
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

fn draw(vehicle: &Vehicle, shown: &mut String) {
    if !is_tty() {
        let line = compact(vehicle);
        if line != *shown {
            println!("{line}");
            let _ = io::stdout().flush();
            *shown = line;
        }
        return;
    }
    let mut frame = String::from("\x1b[H");
    frame.push_str(&fit("Автомобиль  Kalina"));
    frame.push('\n');
    for (index, name) in DAMPERS.iter().enumerate() {
        let command = vehicle.damper_command(index);
        let position = vehicle.damper_position(index);
        let mut line = format!(
            "{:<14} команда {command:.2}   положение {position:.2}",
            name
        );
        if index == 1 {
            line.push_str(&format!("   задание {} °C", celsius_from_unit(command)));
        }
        frame.push_str(&fit(&line));
        frame.push('\n');
    }
    let request = if vehicle.ac_request() {
        "вкл"
    } else {
        "выкл"
    };
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
    print!("{frame}");
    let _ = io::stdout().flush();
}

fn compact(vehicle: &Vehicle) -> String {
    let clutch = if vehicle.clutch { "вкл" } else { "выкл" };
    let request = if vehicle.ac_request() {
        "вкл"
    } else {
        "выкл"
    };
    format!(
        "охлаждение {} °C  задание {} °C  заслонка {:.2}  муфта {clutch}  запрос {request}  вентилятор {:.2}  ошибки {}",
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
