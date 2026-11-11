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
        let attributes = Window::default_attributes()
            .with_title(format!(
                "OpenVaz — {} {}",
                self.vehicle.firmware().family,
                self.vehicle.firmware().id
            ))
            .with_inner_size(LogicalSize::new(DASH_W as f64, DASH_H as f64));
        let window = Arc::new(event_loop.create_window(attributes).expect("окно"));
        let context = softbuffer::Context::new(window.clone()).expect("контекст");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("поверхность");
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(33)));
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.step();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(33)));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
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
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = Some((position.x as f32, position.y as f32));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                let pressed = state == ElementState::Pressed;
                self.mouse_down = pressed;
                if pressed {
                    if let Some((x, y)) = self.pointer {
                        self.click(x, y);
                    }
                }
            }
            _ => {}
        }
        let _ = window;
    }
}

impl DashApp {
    fn step(&mut self) {
        self.pump_socket();
        let pointer = self.pointer_pedal();
        self.vehicle.set_keys(KeyInput {
            gas: if self.hold.gas || pointer.gas { 1.0 } else { 0.0 },
            brake: if self.hold.brake || pointer.brake { 1.0 } else { 0.0 },
            clutch: if self.hold.clutch || pointer.clutch { 1.0 } else { 0.0 },
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
        let Some(stream) = self.stream.as_mut() else { return };
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
        let Some(stream) = self.stream.as_mut() else { return };
        for line in self.vehicle.wire_lines() {
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
            Some(DashHit::Gas) => PointerPedal { gas: true, ..PointerPedal::default() },
            Some(DashHit::Brake) => PointerPedal { brake: true, ..PointerPedal::default() },
            Some(DashHit::Clutch) => PointerPedal { clutch: true, ..PointerPedal::default() },
            Some(DashHit::Starter) => PointerPedal { crank: true, ..PointerPedal::default() },
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
            Key::Named(NamedKey::Escape) => {}
            Key::Character(text) if !event.repeat => match text.as_str() {
                "w" | "W" | "ц" | "Ц" => self.hold.gas = down,
                "s" | "S" | "ы" | "Ы" => self.hold.brake = down,
                "a" | "A" | "ф" | "Ф" => self.hold.clutch = down,
                "1" => {
                    if down {
                        self.hold.gear = Some(Gear::Forward(1));
                    }
                }
                "2" => {
                    if down {
                        self.hold.gear = Some(Gear::Forward(2));
                    }
                }
                "3" => {
                    if down {
                        self.hold.gear = Some(Gear::Forward(3));
                    }
                }
                "4" => {
                    if down {
                        self.hold.gear = Some(Gear::Forward(4));
                    }
                }
                "5" => {
                    if down {
                        self.hold.gear = Some(Gear::Forward(5));
                    }
                }
                "n" | "N" | "т" | "Т" => {
                    if down {
                        self.hold.gear = Some(Gear::Neutral);
                    }
                }
                "r" | "R" | "к" | "К" => {
                    if down {
                        self.hold.gear = Some(Gear::Reverse);
                    }
                }
                "i" | "I" | "ш" | "Ш" => {
                    if down {
                        self.hold.ignition = !self.hold.ignition;
                    }
                }
                _ => {}
            },
            Key::Character(text) => match text.as_str() {
                "w" | "W" | "ц" | "Ц" => self.hold.gas = down,
                "s" | "S" | "ы" | "Ы" => self.hold.brake = down,
                "a" | "A" | "ф" | "Ф" => self.hold.clutch = down,
                _ => {}
            },
            _ => {}
        }
    }

    fn to_dash(&self, x: f32, y: f32) -> (f32, f32) {
        let Some(window) = &self.window else {
            return (x, y);
        };
        let size = window.inner_size();
        Viewport {
            scale: (size.width as f32 / DASH_W).min(size.height as f32 / DASH_H),
            ox: 0.0,
            oy: 0.0,
        }
        .fit_origin(size.width, size.height, DASH_W, DASH_H)
        .to_layout(x, y)
    }

    fn draw(&mut self) {
        let Some(surface) = &mut self.surface else { return };
        let Some(window) = &self.window else { return };
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        if surface
            .resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap())
            .is_err()
        {
            return;
        }
        let pixmap = paint::render_dashboard(&self.vehicle, self.stream.is_some(), &mut self.fonts);
        let view = Viewport {
            scale: 1.0,
            ox: 0.0,
            oy: 0.0,
        }
        .fit_origin(width, height, DASH_W, DASH_H);
        let mut buffer = match surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(_) => return,
        };
        let src_w = pixmap.width() as i32;
        let src_h = pixmap.height() as i32;
        let pixels = pixmap.pixels();
        for y in 0..height {
            for x in 0..width {
                let sx = ((x as f32 - view.ox) / view.scale) as i32;
                let sy = ((y as f32 - view.oy) / view.scale) as i32;
                let (red, green, blue) = if sx >= 0 && sy >= 0 && sx < src_w && sy < src_h {
                    let color = pixels[(sy as u32 * pixmap.width() + sx as u32) as usize].demultiply();
                    (color.red(), color.green(), color.blue())
                } else {
                    (0x12, 0x16, 0x1c)
                };
                buffer[(y * width + x) as usize] = (blue as u32) | ((green as u32) << 8) | ((red as u32) << 16);
            }
        }
        let _ = buffer.present();
    }
}

#[derive(Default)]
struct PointerPedal {
    gas: bool,
    brake: bool,
    clutch: bool,
    crank: bool,
}

impl Viewport {
    fn fit_origin(self, dst_w: u32, dst_h: u32, src_w: f32, src_h: f32) -> Self {
        let dw = dst_w.max(1) as f32;
        let dh = dst_h.max(1) as f32;
        let scale = (dw / src_w).min(dh / src_h);
        Self {
            scale,
            ox: (dw - src_w * scale) * 0.5,
            oy: (dh - src_h * scale) * 0.5,
        }
    }
}
