use env_logger::Env;
use libgsh::{
    async_trait::async_trait,
    cert,
    frame::full_frame_segment,
    r#async::{
        server::AsyncServer,
        service::{AsyncService, AsyncServiceExt},
        Messages,
    },
    shared::{
        protocol::{
            server_hello_ack::{window_settings, FrameFormat, WindowSettings},
            user_input::{window_event::WindowAction, InputEvent},
            Frame, ServerHelloAck,
        },
        ClientEvent,
    },
    tokio,
    tokio_rustls::rustls::{crypto::ring, ServerConfig},
    Result, ServiceError,
};
use std::time::Instant;
use vek::*;

const PIXEL_BYTES: usize = 4; // RGBA
const WINDOW_ID: u32 = 0;
const INITIAL_WIDTH: usize = 300;
const INITIAL_HEIGHT: usize = 300;
const MAX_FPS: u32 = 60;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_line_number(true)
        .format_file(true)
        .format_target(false)
        .format_timestamp(None)
        .init();

    let (key, private_key) = cert::self_signed(&["localhost"]).unwrap();
    ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![key.cert.der().clone()], private_key)
        .unwrap();
    let server = AsyncServer::new(CubeService::default(), config);
    server.serve().await.unwrap();
}

#[derive(Debug, Clone)]
pub struct CubeService {
    start: Instant,
    width: usize,
    height: usize,
    // prev_frame: Vec<u8>,
}

impl Default for CubeService {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            width: INITIAL_WIDTH,
            height: INITIAL_HEIGHT,
            // prev_frame: vec![0; INITIAL_WIDTH * INITIAL_HEIGHT * PIXEL_BYTES],
        }
    }
}

impl CubeService {
    async fn send_frame(&mut self, messages: &mut Messages) -> Result<()> {
        let frame = self.draw_cube(4);
        messages
            .write_message(Frame {
                window_id: WINDOW_ID,
                segments: full_frame_segment(
                    &frame,
                    self.width,
                    self.height,
                    // &mut self.prev_frame,
                    // PIXEL_BYTES,
                ),
                width: self.width as u32,
                height: self.height as u32,
            })
            .await?;
        log::trace!("Frame sent: {}x{}", self.width, self.height);
        Ok(())
    }

    fn draw_cube(&self, stroke_width: usize) -> Vec<u8> {
        let mut frame = vec![0u8; self.width * self.height * PIXEL_BYTES];

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

        // Project vertices
        let projected: Vec<(i32, i32)> = vertices
            .iter()
            .map(|v| {
                let v4 = Vec4::new(v.x, v.y, v.z, 1.0); // Convert Vec3 to Vec4 with w = 1.0
                let transformed = model * v4; // Multiply matrix with vector
                let perspective = 1.5 / (transformed.z + 2.5);
                let x = ((transformed.x * perspective + 0.5) * self.width as f32) as i32;
                let y = ((-transformed.y * perspective + 0.5) * self.height as f32) as i32;
                (x, y)
            })
            .collect();

        // Draw edges
        for (a, b) in edges {
            Self::draw_line(
                projected[a],
                projected[b],
                &mut frame,
                self.width,
                stroke_width,
            );
        }

        frame
    }

    fn draw_line(
        (x0, y0): (i32, i32),
        (x1, y1): (i32, i32),
        frame: &mut [u8],
        width: usize,
        stroke_width: usize,
    ) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);

        loop {
            if x >= 0 && y >= 0 {
                for i in 0..stroke_width {
                    for j in 0..stroke_width {
                        let idx = ((y + i as i32) as usize * width + (x + j as i32) as usize)
                            * PIXEL_BYTES;
                        if idx + 3 < frame.len() {
                            frame[idx] = 255;
                            frame[idx + 1] = 255;
                            frame[idx + 2] = 255;
                            frame[idx + 3] = 255;
                        }
                    }
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

#[async_trait]
impl AsyncService for CubeService {
    async fn main(self, messages: Messages) -> Result<()> {
        <Self as AsyncServiceExt>::main(self, messages).await
    }

    fn server_hello(&self) -> ServerHelloAck {
        ServerHelloAck {
            format: FrameFormat::Rgba.into(),
            windows: vec![WindowSettings {
                window_id: WINDOW_ID,
                monitor_id: None,
                title: "Spinning Cube".into(),
                initial_mode: window_settings::WindowMode::Windowed.into(),
                width: INITIAL_WIDTH as u32,
                height: INITIAL_HEIGHT as u32,
                always_on_top: false,
                allow_resize: true,
                resize_frame: true,
                frame_anchor: window_settings::WindowAnchor::Center.into(),
            }],
            auth_method: None,
        }
    }
}

#[async_trait]
impl AsyncServiceExt for CubeService {
    const MAX_FPS: u32 = MAX_FPS;

    async fn on_startup(&mut self, messages: &mut Messages) -> Result<()> {
        self.send_frame(messages).await
    }

    async fn on_tick(&mut self, messages: &mut Messages) -> Result<()> {
        self.send_frame(messages).await
    }

    async fn on_event(&mut self, messages: &mut Messages, event: ClientEvent) -> Result<()> {
        log::trace!("Got event: {:?}", event);
        if let ClientEvent::UserInput(input) = &event {
            if let InputEvent::WindowEvent(window_event) = input.input_event.unwrap() {
                if window_event.action == WindowAction::Resize as i32 {
                    if input.window_id == WINDOW_ID {
                        self.width = window_event.width as usize;
                        self.height = window_event.height as usize;
                        self.send_frame(messages).await?;
                        log::info!(
                            "WindowEvent: Resize event for window {}: {}x{}",
                            input.window_id,
                            self.width,
                            self.height
                        );
                    } else {
                        log::warn!(
                            "WindowEvent: Resize event for window {} ignored",
                            input.window_id
                        );
                    }
                } else if window_event.action == WindowAction::Close as i32 {
                    return Err(ServiceError::AnyError("Window closed".into()));
                }
            }
        }
        Ok(())
    }
}
