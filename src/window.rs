use std::num::NonZeroU32;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::layout::Viewport;
use crate::model::Session;
use crate::paint::{self, Typeface};

pub fn run(session: Session) {
    let event_loop = EventLoop::new().expect("цикл событий");
    let mut app = App {
        session,
        fonts: Typeface::load(),
        kiosk: std::env::var_os("OPENVAZ_KIOSK").is_some(),
        window: None,
        context: None,
        surface: None,
        pointer: None,
    };
    event_loop.run_app(&mut app).expect("окно");
}

struct App {
    session: Session,
    fonts: Typeface,
    kiosk: bool,
    window: Option<Arc<Window>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    pointer: Option<(f32, f32)>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("OpenVaz")
            .with_inner_size(LogicalSize::new(
                crate::layout::LAYOUT_W as f64,
                crate::layout::LAYOUT_H as f64,
            ));
        let window = Arc::new(event_loop.create_window(attributes).expect("окно"));
        let context = softbuffer::Context::new(window.clone()).expect("контекст");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("поверхность");
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if !self.kiosk && event.logical_key.to_text().as_deref() == Some("\u{1b}") {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(surface) = &mut self.surface {
                    let _ = surface.resize(
                        NonZeroU32::new(size.width.max(1)).unwrap(),
                        NonZeroU32::new(size.height.max(1)).unwrap(),
                    );
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = Some((position.x as f32, position.y as f32));
                if window_mouse_down() {
                    self.move_pointer(position.x as f32, position.y as f32);
                    window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                let Some((x, y)) = self.pointer else { return };
                match state {
                    ElementState::Pressed => {
                        MOUSE_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);
                        self.down(x, y);
                    }
                    ElementState::Released => {
                        MOUSE_DOWN.store(false, std::sync::atomic::Ordering::Relaxed);
                        self.session.pointer_up();
                    }
                }
                window.request_redraw();
            }
            WindowEvent::Touch(touch) => {
                let x = touch.location.x as f32;
                let y = touch.location.y as f32;
                match touch.phase {
                    TouchPhase::Started => self.down(x, y),
                    TouchPhase::Moved => self.move_pointer(x, y),
                    TouchPhase::Ended | TouchPhase::Cancelled => self.session.pointer_up(),
                }
                window.request_redraw();
            }
            _ => {}
        }
    }
}

fn window_mouse_down() -> bool {
    MOUSE_DOWN.load(std::sync::atomic::Ordering::Relaxed)
}

static MOUSE_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl App {
    fn down(&mut self, x: f32, y: f32) {
        let (x, y) = self.to_layout(x, y);
        self.session.pointer_down(x, y);
    }

    fn move_pointer(&mut self, x: f32, y: f32) {
        let (x, y) = self.to_layout(x, y);
        self.session.pointer_move(x, y);
    }

    fn to_layout(&self, x: f32, y: f32) -> (f32, f32) {
        let Some(window) = &self.window else {
            return (x, y);
        };
        let size = window.inner_size();
        Viewport::fit(size.width, size.height).to_layout(x, y)
    }

    fn draw(&mut self) {
        let Some(window) = &self.window else { return };
        let Some(surface) = &mut self.surface else {
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
        let pixmap = paint::render(
            self.session.screen,
            &self.session.climate,
            &self.session.ecu,
            &mut self.fonts,
        );
        let view = Viewport::fit(width, height);
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
                    let color =
                        pixels[(sy as u32 * pixmap.width() + sx as u32) as usize].demultiply();
                    (color.red(), color.green(), color.blue())
                } else {
                    (0x12, 0x16, 0x1c)
                };
                buffer[(y * width + x) as usize] =
                    (blue as u32) | ((green as u32) << 8) | ((red as u32) << 16);
            }
        }
        let _ = buffer.present();
    }
}
