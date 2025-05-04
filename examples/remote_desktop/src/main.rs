use std::{io::Write, time::Instant};

use libgsh::{
    async_trait::async_trait,
    cert,
    frame::full_frame_segment,
    r#async::{
        server::AsyncServer,
        service::{AsyncService, AsyncServiceExt},
        Messages,
    },
    shared::protocol::{
        server_hello_ack::{self, window_settings, FrameFormat, WindowSettings, ZstdCompression},
        Frame, ServerHelloAck,
    },
    tokio,
    tokio_rustls::rustls::ServerConfig,
    ServiceError,
};
use xcap::Monitor;

const FRAME_FORMAT: FrameFormat = FrameFormat::Rgba;
const ZSTD_COMPRESSION_LEVEL: i32 = 3;
const WINDOW_ID: u32 = 0;
const INITIAL_WIDTH: usize = 1920;
const INITIAL_HEIGHT: usize = 1080;
const MAX_FPS: u32 = 60;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
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
    let server = AsyncServer::new(RdpService::default(), config);
    server.serve().await.unwrap();
}

#[derive(Debug, Clone)]
pub struct RdpService {
    last_frame: Instant,
}

impl Default for RdpService {
    fn default() -> Self {
        Self {
            last_frame: Instant::now(),
        }
    }
}

#[async_trait]
impl AsyncService for RdpService {
    fn server_hello(&self) -> libgsh::shared::protocol::ServerHelloAck {
        ServerHelloAck {
            windows: vec![WindowSettings {
                window_id: WINDOW_ID,
                width: INITIAL_WIDTH as u32,
                height: INITIAL_HEIGHT as u32,
                monitor_id: None,
                initial_mode: window_settings::WindowMode::Windowed as i32,
                title: "Remote Desktop".to_string(),
                always_on_top: false,
                allow_resize: true,
                resize_frame: false,
                frame_anchor: window_settings::WindowAnchor::Center as i32,
            }],
            format: FRAME_FORMAT as i32,
            compression: Some(server_hello_ack::Compression::Zstd(ZstdCompression {
                level: ZSTD_COMPRESSION_LEVEL,
            })),
            auth_method: None,
        }
    }

    async fn main(self, messages: Messages) -> libgsh::Result<()> {
        <Self as AsyncServiceExt>::main(self, messages).await
    }
}

#[async_trait]
impl AsyncServiceExt for RdpService {
    async fn on_event(
        &mut self,
        _messages: &mut Messages,
        event: libgsh::shared::ClientEvent,
    ) -> libgsh::Result<()> {
        log::info!("Received event: {:?}", event);
        Ok(())
    }

    async fn on_tick(&mut self, messages: &mut Messages) -> libgsh::Result<()> {
        if self.last_frame.elapsed().as_secs_f32() >= 1.0 / MAX_FPS as f32 {
            messages.write_message(self.get_frame()?).await?;
            self.last_frame = std::time::Instant::now();
            log::debug!("Sent frame");
        }
        Ok(())
    }

    async fn on_startup(&mut self, messages: &mut Messages) -> libgsh::Result<()> {
        messages.write_message(self.get_frame()?).await?;
        log::debug!("Sent initial frame");
        Ok(())
    }
}

impl RdpService {
    fn get_frame(&mut self) -> libgsh::Result<Frame> {
        let monitor = Monitor::all()
            .map_err(|e| ServiceError::Error(format!("{}", e)))?
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .unwrap_or_else(|| Monitor::from_point(0, 0).unwrap());

        let rgba_img = monitor
            .capture_image()
            .map_err(|e| ServiceError::Error(format!("{}", e)))?; // image::DynamicImage
        let (w, h) = rgba_img.dimensions();
        let rgba_vec = rgba_img.into_raw(); // Vec<u8>, len = w*h*4
        log::debug!(
            "Captured image of resolution {}x{} and size: {}",
            w,
            h,
            rgba_vec.len()
        );
        let compressed = self.compress(&rgba_vec, w as usize, h as usize)?;
        log::debug!(
            "Compressed image size: {} (~{:.2}%)",
            compressed.len(),
            compressed.len() as f32 * 100f32 / rgba_vec.len() as f32
        );
        Ok(Frame {
            window_id: WINDOW_ID,
            width: w,
            height: h,
            segments: full_frame_segment(
                &compressed,
                w as usize,
                h as usize,
                // &mut self.previous_frame,
                // 4,
            ),
        })
    }

    fn compress(&self, rgba_vec: &[u8], w: usize, h: usize) -> libgsh::Result<Vec<u8>> {
        let mut encoder = libgsh::zstd::stream::Encoder::new(
            Vec::with_capacity(w * h * 4),
            ZSTD_COMPRESSION_LEVEL,
        )?;
        encoder.write_all(rgba_vec)?;
        Ok(encoder.finish()?)
    }
}
