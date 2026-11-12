use std::io::{Read, Write};
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::engine::Gear;
use crate::layout::Viewport;
use crate::link::{parse_driver_line, parse_panel_line};
use crate::paint::{self, DashHit, Typeface, DASH_H, DASH_W};
use crate::vehicle::{KeyInput, Vehicle};

/// Период кадра окна: 120 Гц.
const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 120);

pub fn run(vehicle: Vehicle, socket: PathBuf) {
    let event_loop = EventLoop::new().expect("цикл событий");
    let mut app = DashApp {
        vehicle,
        socket,
        stream: None,
        incoming: Vec::new(),
        fonts: Typeface::load(),
        window: None,
        context: None,
        surface: None,
        pointer: None,
        mouse_down: false,
        hold: Hold::default(),
        next_frame: Instant::now(),
        last_tick: Instant::now(),
        last_send: Instant::now() - Duration::from_secs(1),
    };
    event_loop.run_app(&mut app).expect("окно");
}

struct DashApp {
    vehicle: Vehicle,
    socket: PathBuf,
    stream: Option<UnixStream>,
    incoming: Vec<u8>,
    fonts: Typeface,
    window: Option<Arc<Window>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    pointer: Option<(f32, f32)>,
    mouse_down: bool,
    hold: Hold,
    next_frame: Instant,
    last_tick: Instant,
    last_send: Instant,
}

struct Hold {
    gas: bool,
    brake: bool,
    clutch: bool,
    crank: bool,
    gear: Option<Gear>,
    ignition: bool,
}

impl Default for Hold {
    fn default() -> Self {
        Self {
            gas: false,
            brake: false,
            clutch: false,
            crank: false,
            gear: None,
            ignition: true,
        }
    }
}

impl ApplicationHandler for DashApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let firmware = self.vehicle.firmware();
        let attributes = Window::default_attributes()
            .with_title(format!(
                "OpenVaz — {} {}",
                firmware.family, firmware.calibration
            ))
            .with_inner_size(LogicalSize::new(DASH_W as f64, DASH_H as f64));
        let window = Arc::new(event_loop.create_window(attributes).expect("окно"));
        let context = softbuffer::Context::new(window.clone()).expect("контекст");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("поверхность");
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if frame_due(&mut self.next_frame, Instant::now()) {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.window.is_none() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => self.on_key(&event),
            WindowEvent::Resized(size) => {
                if let Some(surface) = &mut self.surface {
                    let _ = surface.resize(
                        NonZeroU32::new(size.width.max(1)).unwrap(),
                        NonZeroU32::new(size.height.max(1)).unwrap(),
                    );
                }
            }
            WindowEvent::RedrawRequested => {
                self.step();
                self.draw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = Some((position.x as f32, position.y as f32));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                self.mouse_down = state == ElementState::Pressed;
                if self.mouse_down {
                    if let Some((x, y)) = self.pointer {
                        self.click(x, y);
                    }
                }
            }
            _ => {}
        }
    }
}

impl DashApp {
    fn step(&mut self) {
        self.pump_socket();
        let pointer = self.pointer_pedal();
        self.vehicle.set_keys(KeyInput {
            gas: if self.hold.gas || pointer.gas {
                1.0
            } else {
                0.0
            },
            brake: if self.hold.brake || pointer.brake {
                1.0
            } else {
                0.0
            },
            clutch: if self.hold.clutch || pointer.clutch {
                1.0
            } else {
                0.0
            },
            gear: self.hold.gear,
            ignition: Some(self.hold.ignition),
            crank: self.hold.crank || pointer.crank,
        });
        let dt = self.last_tick.elapsed().as_secs_f32().min(0.1);
        self.vehicle.tick(dt);
        self.last_tick = Instant::now();
        if self.last_send.elapsed() >= Duration::from_millis(200) {
            self.send_state();
            self.last_send = Instant::now();
        }
    }

    fn pump_socket(&mut self) {
        if self.stream.is_none() {
            if let Ok(stream) = UnixStream::connect(&self.socket) {
                let _ = stream.set_nonblocking(true);
                self.stream = Some(stream);
            } else {
                return;
            }
        }
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let mut buf = [0u8; 512];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => {
                    self.stream = None;
                    return;
                }
                Ok(n) => {
                    self.incoming.extend_from_slice(&buf[..n]);
                    if self.incoming.len() > 8192 {
                        self.incoming.clear();
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.stream = None;
                    return;
                }
            }
        }
        while let Some(pos) = self.incoming.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.incoming.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line);
            let text = text.trim();
            if let Some(command) = parse_driver_line(text) {
                self.vehicle.apply_panel(command);
            } else if let Some((channel, value)) = parse_panel_line(text) {
                self.vehicle.apply(&channel, value);
            }
        }
    }

    fn send_state(&mut self) {
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let lines = self
            .vehicle
            .status_lines()
            .into_iter()
            .chain(self.vehicle.motion_lines())
            .chain(self.vehicle.position_lines())
            .chain(self.vehicle.wire_lines());
        for line in lines {
            if writeln!(stream, "{line}").is_err() {
                self.stream = None;
                return;
            }
        }
    }

    fn pointer_pedal(&self) -> PointerPedal {
        if !self.mouse_down {
            return PointerPedal::default();
        }
        let Some((x, y)) = self.pointer else {
            return PointerPedal::default();
        };
        let (x, y) = self.to_dash(x, y);
        match paint::hit_dashboard(x, y) {
            Some(DashHit::Gas) => PointerPedal {
                gas: true,
                ..PointerPedal::default()
            },
            Some(DashHit::Brake) => PointerPedal {
                brake: true,
                ..PointerPedal::default()
            },
            Some(DashHit::Clutch) => PointerPedal {
                clutch: true,
                ..PointerPedal::default()
            },
            Some(DashHit::Starter) => PointerPedal {
                crank: true,
                ..PointerPedal::default()
            },
            _ => PointerPedal::default(),
        }
    }

    fn click(&mut self, x: f32, y: f32) {
        let (x, y) = self.to_dash(x, y);
        match paint::hit_dashboard(x, y) {
            Some(DashHit::Gear(code)) => self.hold.gear = Some(Gear::from_code(code)),
            Some(DashHit::Ignition) => self.hold.ignition = !self.hold.ignition,
            _ => {}
        }
    }

    fn on_key(&mut self, event: &winit::event::KeyEvent) {
        let down = event.state == ElementState::Pressed;
        match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) => self.hold.gas = down,
            Key::Named(NamedKey::ArrowDown) => self.hold.brake = down,
            Key::Named(NamedKey::ArrowLeft) => self.hold.clutch = down,
            Key::Named(NamedKey::Enter) => self.hold.crank = down,
            Key::Character(text) if !event.repeat => self.on_character(text.as_str(), down),
            Key::Character(text) => match text.as_str() {
                "w" | "W" | "ц" | "Ц" => self.hold.gas = down,
                "s" | "S" | "ы" | "Ы" => self.hold.brake = down,
                "a" | "A" | "ф" | "Ф" => self.hold.clutch = down,
                _ => {}
            },
            _ => {}
        }
    }

    fn on_character(&mut self, text: &str, down: bool) {
        match text {
            "w" | "W" | "ц" | "Ц" => self.hold.gas = down,
            "s" | "S" | "ы" | "Ы" => self.hold.brake = down,
            "a" | "A" | "ф" | "Ф" => self.hold.clutch = down,
            "1" if down => self.hold.gear = Some(Gear::Forward(1)),
            "2" if down => self.hold.gear = Some(Gear::Forward(2)),
            "3" if down => self.hold.gear = Some(Gear::Forward(3)),
            "4" if down => self.hold.gear = Some(Gear::Forward(4)),
            "5" if down => self.hold.gear = Some(Gear::Forward(5)),
            "n" | "N" | "т" | "Т" if down => self.hold.gear = Some(Gear::Neutral),
            "r" | "R" | "к" | "К" if down => self.hold.gear = Some(Gear::Reverse),
            "i" | "I" | "ш" | "Ш" if down => self.hold.ignition = !self.hold.ignition,
            _ => {}
        }
    }

    fn to_dash(&self, x: f32, y: f32) -> (f32, f32) {
        let Some(window) = &self.window else {
            return (x, y);
        };
        let size = window.inner_size();
        fit_origin(size.width, size.height).to_layout(x, y)
    }

    fn draw(&mut self) {
        let Some(surface) = &mut self.surface else {
            return;
        };
        let Some(window) = &self.window else {
            return;
        };
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        if surface
            .resize(
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
            )
            .is_err()
        {
            return;
        }
        let pixmap = paint::render_dashboard(&self.vehicle, self.stream.is_some(), &mut self.fonts);
        let view = fit_origin(width, height);
        let mut buffer = match surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(_) => return,
        };
        blit(&mut buffer, width, height, &pixmap, view);
        window.pre_present_notify();
        let _ = buffer.present();
    }
}

fn blit(buffer: &mut [u32], width: u32, height: u32, pixmap: &tiny_skia::Pixmap, view: Viewport) {
    let src_w = pixmap.width();
    let src_h = pixmap.height();
    let src = pixmap.data();
    const BG: u32 = 0x0012_161c;
    if width == src_w && height == src_h && view.scale == 1.0 && view.ox == 0.0 && view.oy == 0.0 {
        let (pixels, _) = src.as_chunks::<4>();
        for (dst, px) in buffer.iter_mut().zip(pixels) {
            *dst = u32::from(px[2]) | (u32::from(px[1]) << 8) | (u32::from(px[0]) << 16);
        }
        return;
    }
    let scale = view.scale.max(f32::EPSILON);
    for y in 0..height {
        let sy = (y as f32 - view.oy) / scale;
        let row = (y * width) as usize;
        if sy < 0.0 || sy >= src_h as f32 {
            buffer[row..row + width as usize].fill(BG);
            continue;
        }
        let src_row = sy as usize * src_w as usize * 4;
        for x in 0..width {
            let sx = (x as f32 - view.ox) / scale;
            buffer[row + x as usize] = if sx < 0.0 || sx >= src_w as f32 {
                BG
            } else {
                let i = src_row + sx as usize * 4;
                u32::from(src[i + 2]) | (u32::from(src[i + 1]) << 8) | (u32::from(src[i]) << 16)
            };
        }
    }
}

#[derive(Default)]
struct PointerPedal {
    gas: bool,
    brake: bool,
    clutch: bool,
    crank: bool,
}

fn fit_origin(dst_w: u32, dst_h: u32) -> Viewport {
    let dw = dst_w.max(1) as f32;
    let dh = dst_h.max(1) as f32;
    let scale = (dw / DASH_W).min(dh / DASH_H);
    Viewport {
        scale,
        ox: (dw - DASH_W * scale) * 0.5,
        oy: (dh - DASH_H * scale) * 0.5,
    }
}

// X11 events can wake the loop before its timer; they must not start another frame.
fn frame_due(next: &mut Instant, now: Instant) -> bool {
    if now < *next {
        return false;
    }
    *next = now + FRAME;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_window_events_do_not_render_frames_or_delay_the_deadline() {
        let start = Instant::now();
        let mut next = start;
        assert!(frame_due(&mut next, start));
        for offset in 0..1000 {
            assert!(!frame_due(&mut next, start + FRAME * offset / 1000));
            assert_eq!(next, start + FRAME);
        }
        assert!(frame_due(&mut next, start + FRAME));
        // A stalled renderer schedules one fresh frame, without replaying old frames.
        let late = start + FRAME * 100;
        assert!(frame_due(&mut next, late));
        assert_eq!(next, late + FRAME);
        assert!(!frame_due(&mut next, late));
    }
}
