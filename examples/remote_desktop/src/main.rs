mod application;
mod frame_source;

use application::RemoteDesktopApplication;
use frame_source::{FrameHub, FrameSource};
use libgsh::{
    async_trait::async_trait,
    server::{GshServer, GshService, GshServiceExt, ServerStream},
    shared::cert,
    shared::frame::full_frame_segment,
    shared::protocol::{
        client_message,
        server_hello_ack::{self, window_settings, FrameFormat, WindowSettings, ZstdCompression},
        Frame, ServerHelloAck,
    },
    tokio,
    tokio_rustls::rustls::ServerConfig,
    ServiceError,
};
use std::{io::Write, time::Instant};
use xcap::Monitor;

const FRAME_FORMAT: FrameFormat = FrameFormat::Rgba;
const ZSTD_COMPRESSION_LEVEL: i32 = 3;
const WINDOW_ID: u32 = 0;
const INITIAL_WIDTH: usize = 480;
const INITIAL_HEIGHT: usize = 270;
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

    let monitor = Monitor::all()
        .unwrap()
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .unwrap_or_else(|| Monitor::from_point(0, 0).unwrap());
    let (video_recorder, video_stream) = monitor
        .video_recorder()
        .expect("Failed to create video recorder");
    video_recorder
        .start()
        .expect("Failed to start video recorder");

    let (frame_hub, capture_forwarder) = FrameHub::from_xcap(video_stream);

    let application = RemoteDesktopApplication::new(frame_hub);
    let server = GshServer::new(RdpService::new(application), config);
    let serve_result = tokio::select! {
        result = server.serve() => result,
        signal = tokio::signal::ctrl_c() => {
            signal.expect("failed to listen for Ctrl-C");
            Ok(())
        }
    };
    video_recorder
        .stop()
        .expect("Failed to stop video recorder");
    capture_forwarder.stop();
    capture_forwarder
        .join()
        .await
        .expect("Screen capture forwarding task failed");
    serve_result.unwrap();
}

pub struct RdpService {
    last_frame: Instant,
    application: RemoteDesktopApplication,
    frames: FrameSource,
}

impl RdpService {
    fn new(application: RemoteDesktopApplication) -> Self {
        let frames = application.subscribe_frames();
        Self {
            last_frame: Instant::now(),
            application,
            frames,
        }
    }
}

impl Clone for RdpService {
    fn clone(&self) -> Self {
        Self::new(self.application.clone())
    }
}

#[async_trait]
impl GshService for RdpService {
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

    async fn main(self, stream: ServerStream) -> libgsh::Result<()> {
        <Self as GshServiceExt>::main(self, stream).await
    }
}

#[async_trait]
impl GshServiceExt for RdpService {
    async fn on_event(
        &mut self,
        _stream: &mut ServerStream,
        event: client_message::ClientEvent,
    ) -> libgsh::Result<()> {
        if let client_message::ClientEvent::UserInput(input) = event {
            self.application.handle_input(&input);
        }
        Ok(())
    }

    async fn on_tick(&mut self, stream: &mut ServerStream) -> libgsh::Result<()> {
        if self.last_frame.elapsed().as_secs_f32() >= 1.0 / MAX_FPS as f32 {
            stream.send(self.get_frame().await?).await?;
            self.last_frame = std::time::Instant::now();
            log::debug!("Sent frame");
        }
        Ok(())
    }

    async fn on_startup(&mut self, stream: &mut ServerStream) -> libgsh::Result<()> {
        stream.send(self.get_frame().await?).await?;
        log::debug!("Sent initial frame");
        Ok(())
    }
}

impl RdpService {
    async fn get_frame(&mut self) -> libgsh::Result<Frame> {
        let frame = self
            .frames
            .next()
            .await
            .ok_or_else(|| ServiceError::Error("Screen capture stream closed".to_string()))?;

        log::debug!(
            "Captured image of resolution {}x{} and size: {}",
            frame.width,
            frame.height,
            frame.rgba.len()
        );
        let compressed = self.compress(&frame.rgba, frame.width as usize, frame.height as usize)?;
        log::debug!(
            "Compressed image size: {} (~{:.2}%)",
            compressed.len(),
            compressed.len() as f32 * 100f32 / frame.rgba.len() as f32
        );
        Ok(Frame {
            window_id: WINDOW_ID,
            width: frame.width,
            height: frame.height,
            segments: full_frame_segment(&compressed, frame.width as usize, frame.height as usize),
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
