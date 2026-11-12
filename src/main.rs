use std::path::Path;

use openvaz::model::{writable_state_dir, Screen, Session};
use openvaz::paint::{render, write_ppm, Typeface};

fn main() {
    let mut framebuffer = None;
    let mut dump = None;
    let mut screen = None;
    let mut car_socket = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fb" => framebuffer = args.next(),
            "--dump-ppm" => dump = args.next(),
            "--screen" => screen = args.next(),
            "--car-socket" => {
                car_socket = Some(args.next().unwrap_or_else(|| {
                    eprintln!("после --car-socket нужен путь к сокету");
                    std::process::exit(2);
                }));
            }
            "--help" => {
                println!(
                    "openvaz [--fb /dev/fb0] [--car-socket путь] [--dump-ppm кадр.ppm] [--screen climate|ecu|readings|drive]"
                );
                println!("OPENVAS_OUTPUT=hardware включает PCA9685, пин 7 и ШИМ пина 8.");
                println!("Калибровка заслонок: файл servos в каталоге настроек.");
                return;
            }
            other => {
                eprintln!("неизвестный аргумент {other}");
                std::process::exit(2);
            }
        }
    }

    let mut session = Session::boot(writable_state_dir());
    session.screen = match screen.as_deref() {
        Some("ecu") => Screen::Ecu,
        Some("readings") => Screen::Readings,
        Some("drive") => Screen::Drive,
        _ => Screen::Climate,
    };

    if let Some(path) = dump {
        let mut fonts = Typeface::load();
        let pixmap = render(session.screen, &session.climate, &session.ecu, &mut fonts);
        write_ppm(Path::new(&path), &pixmap).expect("кадр");
        return;
    }

    if let Some(path) = framebuffer {
        openvaz::fb::run(&path, session);
        return;
    }

    #[cfg(feature = "window")]
    {
        if car_socket.is_some() || std::env::var_os("OPENVAZ_CAR").is_none() {
            let path = car_socket.map(std::path::PathBuf::from).unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("emu/build/car.sock")
            });
            session.listen_car(&path).unwrap_or_else(|error| {
                eprintln!(
                    "не открыть сокет {}: {error}; закройте другую панель или QEMU",
                    path.display()
                );
                std::process::exit(1);
            });
        }
        openvaz::window::run(session);
    }

    #[cfg(not(feature = "window"))]
    {
        let _ = (session, car_socket);
        eprintln!("эта сборка без окна: укажите --fb или --dump-ppm");
        std::process::exit(1);
    }
}
