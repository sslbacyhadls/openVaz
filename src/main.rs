use std::path::Path;

use openvaz::model::{writable_state_dir, Screen, Session};
use openvaz::paint::{render, write_ppm, Typeface};

fn main() {
    let mut framebuffer = None;
    let mut dump = None;
    let mut screen = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fb" => framebuffer = args.next(),
            "--dump-ppm" => dump = args.next(),
            "--screen" => screen = args.next(),
            "--help" => {
                println!("openvaz [--fb /dev/fb0] [--dump-ppm кадр.ppm] [--screen climate|ecu]");
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
    if screen.as_deref() == Some("ecu") {
        session.screen = Screen::Ecu;
    }

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
    openvaz::window::run(session);

    #[cfg(not(feature = "window"))]
    {
        let _ = session;
        eprintln!("эта сборка без окна: укажите --fb или --dump-ppm");
        std::process::exit(1);
    }
}
