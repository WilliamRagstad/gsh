use libgsh::{
    async_trait::async_trait,
    server::{GshServer, GshService, GshServiceExt, ServerStream},
    shared::frame::full_frame_segment,
    shared::protocol::{client_message::ClientEvent, *},
    tokio_rustls::rustls::ServerConfig,
    Result,
};
use std::time::Instant;

/// A simple benchmark server that can generate various load patterns
#[derive(Debug, Clone)]
pub struct BenchmarkServer {
    pub start_time: Instant,
    pub frame_count: u64,
    pub width: usize,
    pub height: usize,
}

impl Default for BenchmarkServer {
    fn default() -> Self {
        Self {
            start_time: Instant::now(),
            frame_count: 0,
            width: 300,
            height: 300,
        }
    }
}

impl BenchmarkServer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            start_time: Instant::now(),
            frame_count: 0,
            width,
            height,
        }
    }

    pub fn create_async_server(self, config: ServerConfig) -> GshServer<Self> {
        GshServer::new(self, config)
    }

    async fn send_frame(&mut self, messages: &mut ServerStream) -> Result<()> {
        self.frame_count += 1;

        let mut frame_data = vec![0u8; self.width * self.height * 4]; // RGBA
        for y in 0..self.height {
            for x in 0..self.width {
                let idx = (y * self.width + x) * 4;
                frame_data[idx] = (x * 255 / self.width) as u8; // R
                frame_data[idx + 1] = (y * 255 / self.height) as u8; // G
                frame_data[idx + 2] = 128; // B
                frame_data[idx + 3] = 255; // A
            }
        }

        let frame = Frame {
            window_id: 0,
            segments: full_frame_segment(&frame_data, self.width, self.height),
            width: self.width as u32,
            height: self.height as u32,
        };

        messages.send(frame).await?;
        messages.flush().await?;
        Ok(())
    }
}

#[async_trait]
impl GshService for BenchmarkServer {
    async fn main(self, messages: ServerStream) -> Result<()> {
        <Self as GshServiceExt>::main(self, messages).await
    }

    fn server_hello(&self) -> ServerHelloAck {
        ServerHelloAck {
            format: server_hello_ack::FrameFormat::Rgba.into(),
            compression: None,
            windows: vec![server_hello_ack::WindowSettings {
                window_id: 0,
                monitor_id: None,
                title: "Benchmark Server".to_string(),
                initial_mode: server_hello_ack::window_settings::WindowMode::Windowed.into(),
                width: self.width as u32,
                height: self.height as u32,
                always_on_top: false,
                allow_resize: false,
                resize_frame: false,
                frame_anchor: server_hello_ack::window_settings::WindowAnchor::Center.into(),
            }],
            auth_method: None,
        }
    }
}

#[async_trait]
impl GshServiceExt for BenchmarkServer {
    const MAX_FPS: u32 = 60;

    async fn on_startup(&mut self, messages: &mut ServerStream) -> Result<()> {
        self.send_frame(messages).await
    }

    async fn on_tick(&mut self, messages: &mut ServerStream) -> Result<()> {
        self.send_frame(messages).await
    }

    async fn on_event(&mut self, _messages: &mut ServerStream, event: ClientEvent) -> Result<()> {
        log::trace!("Received event: {:?}", event);
        Ok(())
    }
}
