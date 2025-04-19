use env_logger::Env;
use libgsh::{
    cert,
    rustls::ServerConfig,
    shared::{
        protocol::{
            self,
            user_input::{window_event::WindowAction, InputEvent},
            window_settings, Frame, ServerHelloAck, WindowSettings,
        },
        ClientEvent,
    },
    simple::{
        server::{Messages, SimpleServer},
        service::{Result, SerivceError, SimpleService, SimpleServiceExt},
    },
};
use std::time::Instant;
use vek::*;

const PIXEL_BYTES: usize = 4; // RGBA
const WINDOW_ID: u32 = 0;
const INITIAL_WIDTH: u32 = 50;
const INITIAL_HEIGHT: u32 = 50;

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_line_number(true)
        .format_file(true)
        .format_target(false)
        .format_timestamp(None)
        .init();

    let (key, private_key) = cert::self_signed(&["localhost"]).unwrap();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![key.cert.der().clone()], private_key)
        .unwrap();
    let server: SimpleServer<CubeService> = SimpleServer::new(config);
    server.serve().unwrap();
}

pub struct CubeService {
    start: Instant,
    width: u32,
    height: u32,
}

impl CubeService {
    fn send_frame(&self, messages: &mut Messages) -> Result<()> {
        let frame = self.draw_cube();
        messages.write_message(Frame {
            window_id: WINDOW_ID,
            data: frame,
            width: self.width,
            height: self.height,
        })?;
        Ok(())
    }

    fn draw_cube(&self) -> Vec<u8> {
        let width = self.width as usize;
        let height = self.height as usize;
        let mut frame = vec![0u8; width * height * PIXEL_BYTES];

        // Define cube vertices
        let size = 0.4;
        let vertices = [
            Vec3::new(-size, -size, -size),
            Vec3::new(size, -size, -size),
            Vec3::new(size, size, -size),
            Vec3::new(-size, size, -size),
            Vec3::new(-size, -size, size),
            Vec3::new(size, -size, size),
            Vec3::new(size, size, size),
            Vec3::new(-size, size, size),
        ];

        let edges = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ];

        // Rotation based on time
        let angle: f32 = self.start.elapsed().as_secs_f32();
        let rot_x: Mat4<f32> = Mat4::rotation_x(angle);
        let rot_y: Mat4<f32> = Mat4::rotation_y(angle * 0.7);
        let model: Mat4<f32> = rot_y * rot_x;

        log::trace!("Angle: {}", angle);
        log::trace!("Rotation X: {:?}", rot_x);
        log::trace!("Rotation Y: {:?}", rot_y);
        log::trace!("Model Matrix: {:?}", model);

        // Project vertices
        let projected: Vec<(i32, i32)> = vertices
            .iter()
            .map(|v| {
                let v4 = Vec4::new(v.x, v.y, v.z, 1.0); // Konvertera Vec3 till Vec4 med w = 1.0
                let transformed = model * v4; // Multiplicera matrisen med vektorn
                let perspective = 1.5 / (transformed.z + 2.5);
                let x = ((transformed.x * perspective + 0.5) * self.width as f32) as i32;
                let y = ((-transformed.y * perspective + 0.5) * self.height as f32) as i32;
                (x, y)
            })
            .collect();

        // Draw edges
        for (a, b) in edges {
            Self::draw_line(projected[a], projected[b], &mut frame, width);
        }

        frame
    }

    fn draw_line((x0, y0): (i32, i32), (x1, y1): (i32, i32), frame: &mut [u8], width: usize) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);

        loop {
            if x >= 0 && y >= 0 {
                let idx = (y as usize * width + x as usize) * PIXEL_BYTES;
                if idx + 3 < frame.len() {
                    frame[idx] = 255;
                    frame[idx + 1] = 255;
                    frame[idx + 2] = 255;
                    frame[idx + 3] = 255;
                }
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }
}

impl SimpleService for CubeService {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            width: INITIAL_WIDTH,
            height: INITIAL_HEIGHT,
        }
    }

    fn main(self, messages: Messages) -> Result<()> {
        <Self as SimpleServiceExt>::main(self, messages)
    }

    fn server_hello() -> ServerHelloAck {
        ServerHelloAck {
            format: protocol::FrameFormat::Rgba.into(),
            windows: vec![WindowSettings {
                window_id: WINDOW_ID,
                title: "Spinning Cube".into(),
                initial_mode: window_settings::WindowMode::Windowed.into(),
                width: INITIAL_WIDTH,
                height: INITIAL_HEIGHT,
                always_on_top: false,
                allow_resize: true,
                resize_frame: true,
                anchor: window_settings::FrameAnchor::Center.into(),
            }],
        }
    }
}

impl SimpleServiceExt for CubeService {
    const FPS: u32 = 10; // 1 FPS for simplicity

    fn on_startup(&mut self, messages: &mut Messages) -> Result<()> {
        self.send_frame(messages)
    }

    fn on_tick(&mut self, messages: &mut Messages) -> Result<()> {
        self.send_frame(messages)
    }

    fn on_event(&mut self, messages: &mut Messages, event: ClientEvent) -> Result<()> {
        if let ClientEvent::UserInput(input) = &event {
            if let InputEvent::WindowEvent(window_event) = input.input_event.unwrap() {
                if window_event.action == WindowAction::Resize as i32 {
                    if input.window_id == WINDOW_ID {
                        self.width = window_event.width;
                        self.height = window_event.height;
                        self.send_frame(messages)?;
                    } else {
                        log::warn!(
                            "WindowEvent: Resize event for window {} ignored",
                            input.window_id
                        );
                    }
                } else if window_event.action == WindowAction::Close as i32 {
                    return Err(SerivceError::AnyError("Window closed".into()));
                }
            }
        }
        Ok(())
    }
}
