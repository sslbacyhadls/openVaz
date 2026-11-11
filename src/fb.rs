use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::ptr;

use tiny_skia::Pixmap;

use crate::layout::Viewport;
use crate::model::{Screen, Session};
use crate::paint::{self, Typeface};

const FBIOGET_VSCREENINFO: u32 = 0x4600;
const FBIOPUT_VSCREENINFO: u32 = 0x4601;
const FBIOGET_FSCREENINFO: u32 = 0x4602;

fn ioctl_req(value: u32) -> libc::Ioctl {
    value as libc::Ioctl
}

#[repr(C)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
struct FbVarScreeninfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
struct FbFixScreeninfo {
    id: [u8; 16],
    smem_start: usize,
    smem_len: u32,
    type_: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: usize,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

#[repr(C)]
struct InputEvent {
    sec: i64,
    usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const BTN_LEFT: u16 = 0x110;
const BTN_TOUCH: u16 = 0x14a;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;

pub fn run(path: &str, mut session: Session) {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap_or_else(|error| panic!("не открыть {path}: {error}"));
    let fd = file.as_raw_fd();
    let mut var = read_var(fd);
    try_mode(&mut var, fd);
    let fix = read_fix(fd);
    if var.bits_per_pixel != 32 {
        eprintln!(
            "openvaz: глубина цвета {}, нужна 32",
            var.bits_per_pixel
        );
        std::process::exit(1);
    }
    let width = var.xres;
    let height = var.yres;
    let stride = fix.line_length as usize;
    let map_len = fix.smem_len as usize;
    let frame = unsafe {
        libc::mmap(
            ptr::null_mut(),
            map_len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if frame == libc::MAP_FAILED {
        eprintln!("openvaz: кадр не отображён");
        std::process::exit(1);
    }
    let mut fonts = Typeface::load();
    let mut devices = open_inputs();
    let mut dirty = true;
    let mut down = false;
    loop {
        if dirty {
            let pixmap = paint::render(session.screen, &session.climate, &session.ecu, &mut fonts);
            blit(&pixmap, frame as *mut u8, width, height, stride, &var);
            dirty = false;
        }
        let _ready = wait_io(&devices, session.car_fd());
        let live = matches!(session.screen, Screen::Ecu | Screen::Readings | Screen::Drive);
        if session.service_car() && live {
            dirty = true;
        }
        if session.screen == Screen::Drive {
            dirty = true;
        }
        for device in &mut devices {
            if device.drain(&mut session, &mut down, width, height) {
                dirty = true;
            }
        }
    }
}

fn read_var(fd: i32) -> FbVarScreeninfo {
    let mut info: FbVarScreeninfo = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(fd, ioctl_req(FBIOGET_VSCREENINFO), &mut info) };
    if rc < 0 {
        eprintln!("openvaz: режим экрана не прочитан");
        std::process::exit(1);
    }
    info
}

fn try_mode(info: &mut FbVarScreeninfo, fd: i32) {
    let width = crate::layout::LAYOUT_W as u32;
    let height = crate::layout::LAYOUT_H as u32;
    info.xres = width;
    info.yres = height;
    info.xres_virtual = info.xres_virtual.max(width);
    info.yres_virtual = info.yres_virtual.max(height);
    let rc = unsafe { libc::ioctl(fd, ioctl_req(FBIOPUT_VSCREENINFO), info as *mut FbVarScreeninfo) };
    if rc < 0 {
        *info = read_var(fd);
        eprintln!(
            "openvaz: экран {}x{}, режим {width}x{height} драйвер не принял",
            info.xres, info.yres
        );
    }
}

fn read_fix(fd: i32) -> FbFixScreeninfo {
    let mut info: FbFixScreeninfo = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(fd, ioctl_req(FBIOGET_FSCREENINFO), &mut info) };
    if rc < 0 {
        eprintln!("openvaz: шаг строки экрана не прочитан");
        std::process::exit(1);
    }
    info
}

struct Device {
    fd: i32,
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
    absolute: bool,
    x: i32,
    y: i32,
    touch: bool,
}

fn open_inputs() -> Vec<Device> {
    let mut devices = Vec::new();
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return devices;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("event") {
            continue;
        }
        let Ok(file) = OpenOptions::new().read(true).open(entry.path()) else {
            continue;
        };
        let fd = file.as_raw_fd();
        let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) };
        let (min_x, max_x) = abs_range(fd, ABS_X).unwrap_or((0, 32767));
        let (min_y, max_y) = abs_range(fd, ABS_Y).unwrap_or((0, 32767));
        let absolute = abs_range(fd, ABS_X).is_some();
        // Файл остаётся открытым: fd живёт, пока процесс не завершится.
        std::mem::forget(file);
        devices.push(Device {
            fd,
            min_x,
            max_x,
            min_y,
            max_y,
            absolute,
            x: 0,
            y: 0,
            touch: false,
        });
    }
    devices
}

fn abs_range(fd: i32, axis: u16) -> Option<(i32, i32)> {
    #[repr(C)]
    struct AbsInfo {
        value: i32,
        minimum: i32,
        maximum: i32,
        fuzz: i32,
        flat: i32,
        resolution: i32,
    }
    let mut info = AbsInfo {
        value: 0,
        minimum: 0,
        maximum: 0,
        fuzz: 0,
        flat: 0,
        resolution: 0,
    };
    let request = ior(b'E', 0x40 + axis as u32, std::mem::size_of::<AbsInfo>() as u32);
    let rc = unsafe { libc::ioctl(fd, ioctl_req(request), &mut info) };
    if rc < 0 || info.maximum <= info.minimum {
        None
    } else {
        Some((info.minimum, info.maximum))
    }
}

fn ior(type_: u8, nr: u32, size: u32) -> u32 {
    (2u32 << 30) | (size << 16) | ((type_ as u32) << 8) | nr
}

fn wait_io(devices: &[Device], car: Option<i32>) -> bool {
    let mut polls: Vec<libc::pollfd> = devices
        .iter()
        .map(|device| libc::pollfd {
            fd: device.fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    if let Some(fd) = car {
        polls.push(libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        });
    }
    if polls.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(200));
        return false;
    }
    let rc = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, 200) };
    rc > 0
}

impl Device {
    fn drain(&mut self, session: &mut Session, down: &mut bool, width: u32, height: u32) -> bool {
        let mut dirty = false;
        loop {
            let mut event = InputEvent {
                sec: 0,
                usec: 0,
                type_: 0,
                code: 0,
                value: 0,
            };
            let n = unsafe {
                libc::read(
                    self.fd,
                    &mut event as *mut InputEvent as *mut libc::c_void,
                    std::mem::size_of::<InputEvent>(),
                )
            };
            if n < 0 {
                break;
            }
            if n as usize != std::mem::size_of::<InputEvent>() {
                continue;
            }
            match event.type_ {
                EV_ABS if event.code == ABS_X => self.x = event.value,
                EV_ABS if event.code == ABS_Y => self.y = event.value,
                EV_KEY if event.code == BTN_TOUCH || event.code == BTN_LEFT => {
                    self.touch = event.value != 0;
                }
                EV_SYN if event.code == SYN_REPORT => {
                    if !self.absolute {
                        continue;
                    }
                    let view = Viewport::fit(width, height);
                    let (x, y) = view.to_layout(self.screen_x(width), self.screen_y(height));
                    if self.touch && !*down {
                        session.pointer_down(x, y);
                        *down = true;
                        dirty = true;
                    } else if self.touch && *down {
                        session.pointer_move(x, y);
                        dirty = true;
                    } else if !self.touch && *down {
                        session.pointer_up();
                        *down = false;
                        dirty = true;
                    }
                }
                _ => {}
            }
        }
        dirty
    }

    fn screen_x(&self, width: u32) -> f32 {
        scale_axis(self.x, self.min_x, self.max_x, width)
    }

    fn screen_y(&self, height: u32) -> f32 {
        scale_axis(self.y, self.min_y, self.max_y, height)
    }
}

fn scale_axis(value: i32, min: i32, max: i32, span: u32) -> f32 {
    let range = (max - min).max(1) as f32;
    ((value - min) as f32 / range).clamp(0.0, 1.0) * (span.saturating_sub(1) as f32)
}

fn blit(
    pixmap: &Pixmap,
    frame: *mut u8,
    width: u32,
    height: u32,
    stride: usize,
    info: &FbVarScreeninfo,
) {
    let view = Viewport::fit(width, height);
    let src_w = pixmap.width() as i32;
    let src_h = pixmap.height() as i32;
    let pixels = pixmap.pixels();
    for y in 0..height {
        for x in 0..width {
            let sx = ((x as f32 - view.ox) / view.scale) as i32;
            let sy = ((y as f32 - view.oy) / view.scale) as i32;
            let (red, green, blue) = if sx >= 0 && sy >= 0 && sx < src_w && sy < src_h {
                let color = pixels[(sy * src_w + sx) as usize].demultiply();
                (color.red(), color.green(), color.blue())
            } else {
                (0x12, 0x16, 0x1c)
            };
            let packed = ((red as u32) << info.red.offset)
                | ((green as u32) << info.green.offset)
                | ((blue as u32) << info.blue.offset);
            unsafe {
                let pixel = frame.add(y as usize * stride + x as usize * 4) as *mut u32;
                *pixel = packed;
            }
        }
    }
}

