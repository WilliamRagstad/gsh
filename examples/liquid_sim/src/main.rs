use env_logger::Env;
use glam::Vec2;
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
            server_hello_ack::{window_settings, Compression, FrameFormat, WindowSettings, ZstdCompression},
            user_input::{window_event::WindowAction, InputEvent},
            Frame, ServerHelloAck,
        },
        ClientEvent,
    },
    tokio,
    tokio_rustls::rustls::{crypto::ring, ServerConfig},
    Result, ServiceError,
};
use ndarray::Array2;
use rayon::prelude::*;
use std::time::Instant;

const WINDOW_ID: u32 = 0;
const INITIAL_WIDTH: usize = 512;
const INITIAL_HEIGHT: usize = 512;
const MAX_FPS: u32 = 60;
const PIXEL_BYTES: usize = 4; // RGBA8
const ZSTD_COMPRESSION_LEVEL: i32 = 3;

// Particle data structure for the simulation
#[derive(Copy, Clone, Debug)]
struct Particle {
    position: Vec2,
    velocity: Vec2,
}



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

    let service = LiquidSimService::default();
    let server = AsyncServer::new(service, config);
    server.serve().await.unwrap();
}

#[derive(Debug, Clone)]
pub struct LiquidSimService {
    particles: Vec<Particle>,
    width: usize,
    height: usize,
    last_update: Instant,
}

impl Default for LiquidSimService {
    fn default() -> Self {
        Self {
            particles: Self::init_particles(2048, INITIAL_WIDTH, INITIAL_HEIGHT),
            width: INITIAL_WIDTH,
            height: INITIAL_HEIGHT,
            last_update: Instant::now(),
        }
    }
}

impl LiquidSimService {

    fn init_particles(count: usize, width: usize, height: usize) -> Vec<Particle> {
        use std::f32::consts::PI;
        let mut particles = Vec::with_capacity(count);
        let center = Vec2::new(width as f32 * 0.5, height as f32 * 0.5);

        for i in 0..count {
            let angle = (i as f32 / count as f32) * 2.0 * PI;
            let radius = (i as f32 / count as f32) * (width.min(height) as f32 * 0.3);
            let pos = center + Vec2::new(angle.cos() * radius, angle.sin() * radius);

            // Velocity pointing outward with some tangential component
            let vel = Vec2::new(
                angle.cos() * 50.0 + angle.sin() * 20.0,
                angle.sin() * 50.0 - angle.cos() * 20.0,
            );

            particles.push(Particle {
                position: pos,
                velocity: vel,
            });
        }

        particles
    }

    fn update_particles(&mut self, dt: f32) {
        let width = self.width as f32;
        let height = self.height as f32;
        
        // Update each particle in parallel using rayon
        self.particles.par_iter_mut().for_each(|particle| {
            // Apply gravity
            let gravity = Vec2::new(0.0, 200.0);
            particle.velocity += gravity * dt;

            // Apply damping
            particle.velocity *= 0.995;

            // Update position
            particle.position += particle.velocity * dt;

            // Bounce off walls
            let margin = 5.0;
            if particle.position.x < margin {
                particle.position.x = margin;
                particle.velocity.x = particle.velocity.x.abs() * 0.8;
            }
            if particle.position.x > width - margin {
                particle.position.x = width - margin;
                particle.velocity.x = -particle.velocity.x.abs() * 0.8;
            }
            if particle.position.y < margin {
                particle.position.y = margin;
                particle.velocity.y = particle.velocity.y.abs() * 0.8;
            }
            if particle.position.y > height - margin {
                particle.position.y = height - margin;
                particle.velocity.y = -particle.velocity.y.abs() * 0.8;
            }
        });

        // Compute inter-particle forces (simplified for performance)
        // We'll use a simple n^2 approach but with spatial optimization
        let particles_clone = self.particles.clone();
        let influence_radius = 20.0;
        let repulsion_strength = 5000.0;

        self.particles.par_iter_mut().enumerate().for_each(|(i, particle)| {
            let mut force = Vec2::ZERO;

            for (j, other) in particles_clone.iter().enumerate() {
                if i == j {
                    continue;
                }

                let diff = particle.position - other.position;
                let dist_sq = diff.length_squared();

                if dist_sq < influence_radius * influence_radius && dist_sq > 0.01 {
                    let dist = dist_sq.sqrt();
                    let dir = diff / dist;
                    let repulsion = repulsion_strength / dist_sq;
                    force += dir * repulsion;
                }
            }

            particle.velocity += force * dt;
        });
    }

    fn render_particles(&self) -> Vec<u8> {
        // Create a 2D array for the frame buffer
        let mut frame = Array2::<u32>::zeros((self.height, self.width));

        // Render each particle
        for particle in &self.particles {
            let x = particle.position.x as usize;
            let y = particle.position.y as usize;

            if x < self.width && y < self.height {
                // Calculate color based on velocity
                let speed = particle.velocity.length();
                let normalized_speed = (speed / 200.0).clamp(0.0, 1.0);

                // Blue to cyan to white based on speed
                let r = (0.2 + normalized_speed * 0.8 * 255.0) as u8;
                let g = (0.4 + normalized_speed * 0.6 * 255.0) as u8;
                let b = (0.8 + normalized_speed * 0.2 * 255.0) as u8;
                let a = 230u8;

                // Pack RGBA into u32
                let color = (a as u32) << 24 | (b as u32) << 16 | (g as u32) << 8 | (r as u32);

                // Draw a small circle (simplified as a square for performance)
                let size = 3;
                for dy in 0..size {
                    for dx in 0..size {
                        let px = x.saturating_add(dx).saturating_sub(size / 2);
                        let py = y.saturating_add(dy).saturating_sub(size / 2);
                        if px < self.width && py < self.height {
                            frame[[py, px]] = color;
                        }
                    }
                }
            }
        }

        // Convert to RGBA8 byte array
        let mut rgba_data = Vec::with_capacity(self.width * self.height * PIXEL_BYTES);
        for pixel in frame.iter() {
            // Unpack u32 back to RGBA
            let r = (*pixel & 0xFF) as u8;
            let g = ((*pixel >> 8) & 0xFF) as u8;
            let b = ((*pixel >> 16) & 0xFF) as u8;
            let a = ((*pixel >> 24) & 0xFF) as u8;

            // Fill background if pixel is transparent
            if a == 0 {
                rgba_data.extend_from_slice(&[5, 5, 12, 255]); // Dark blue background
            } else {
                rgba_data.extend_from_slice(&[r, g, b, a]);
            }
        }

        rgba_data
    }

    fn simulate_and_render(&mut self) -> Vec<u8> {
        // Update delta time
        let dt = self.last_update.elapsed().as_secs_f32();
        self.last_update = Instant::now();

        // Update particle physics
        self.update_particles(dt);

        // Render to RGBA buffer
        self.render_particles()
    }

    async fn send_frame(&mut self, messages: &mut Messages) -> Result<()> {
        let rgba_data = self.simulate_and_render();

        // Compress the data with Zstd
        use std::io::Write;
        let mut encoder = libgsh::zstd::stream::Encoder::new(
            Vec::with_capacity(self.width * self.height * PIXEL_BYTES),
            ZSTD_COMPRESSION_LEVEL,
        )?;
        encoder.write_all(&rgba_data)?;
        let compressed = encoder.finish()?;

        log::debug!(
            "Frame: {}x{}, uncompressed: {} bytes, compressed: {} bytes ({:.1}% compression)",
            self.width,
            self.height,
            rgba_data.len(),
            compressed.len(),
            (compressed.len() as f32 / rgba_data.len() as f32) * 100.0
        );

        messages
            .write_message(Frame {
                window_id: WINDOW_ID,
                segments: full_frame_segment(&compressed, self.width, self.height),
                width: self.width as u32,
                height: self.height as u32,
            })
            .await?;

        Ok(())
    }

    fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        // Reinitialize particles for new dimensions
        self.particles = Self::init_particles(self.particles.len(), width, height);
    }
}

#[async_trait]
impl AsyncService for LiquidSimService {
    async fn main(self, messages: Messages) -> Result<()> {
        <Self as AsyncServiceExt>::main(self, messages).await
    }

    fn server_hello(&self) -> ServerHelloAck {
        ServerHelloAck {
            format: FrameFormat::Rgba.into(),
            compression: Some(Compression::Zstd(ZstdCompression {
                level: ZSTD_COMPRESSION_LEVEL,
            })),
            windows: vec![WindowSettings {
                window_id: WINDOW_ID,
                monitor_id: None,
                title: "Liquid Simulation".into(),
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
impl AsyncServiceExt for LiquidSimService {
    const MAX_FPS: u32 = MAX_FPS;

    async fn on_startup(&mut self, messages: &mut Messages) -> Result<()> {
        log::info!("Starting liquid simulation...");
        self.send_frame(messages).await
    }

    async fn on_tick(&mut self, messages: &mut Messages) -> Result<()> {
        self.send_frame(messages).await
    }

    async fn on_event(&mut self, messages: &mut Messages, event: ClientEvent) -> Result<()> {
        if let ClientEvent::UserInput(input) = &event {
            if let Some(InputEvent::WindowEvent(window_event)) = input.input_event.as_ref() {
                if window_event.action == WindowAction::Resize as i32 {
                    if input.window_id == WINDOW_ID {
                        let new_width = window_event.width as usize;
                        let new_height = window_event.height as usize;
                        log::info!("Resizing to {}x{}", new_width, new_height);
                        self.resize(new_width, new_height);
                        self.send_frame(messages).await?;
                    }
                } else if window_event.action == WindowAction::Close as i32 {
                    return Err(ServiceError::AnyError("Window closed".into()));
                }
            }
        }
        Ok(())
    }
}
