use libgsh::{
    async_trait::async_trait,
    server::{GshServer, GshService, GshServiceExt, GshStream},
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
use std::{
    io::Write,
    sync::{mpsc::Receiver, Arc, Mutex},
    time::Instant,
};
use xcap::Monitor;

#[derive(Debug, Clone)]
pub struct XCapFrame {
    pub width: u32,
    pub height: u32,
    pub raw: Vec<u8>,
}

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

    // Setup recorder
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

    // Unsafe transmute to convert video_stream Recorder to XCapFrame receiver
    // Ugly hack because `xcap::video_recorder::Frame` is not public.
    let video_stream: Receiver<XCapFrame> = unsafe { std::mem::transmute(video_stream) };
    let recorder = Arc::new(Mutex::new(video_stream));

    // Start service
    let server = GshServer::new(RdpService::new(recorder), config);
    server.serve().await.unwrap();
}

#[derive(Debug, Clone)]
pub struct RdpService {
    last_frame: Instant,
    recorder: Arc<Mutex<Receiver<XCapFrame>>>,
}

impl RdpService {
    fn new(recorder: Arc<Mutex<Receiver<XCapFrame>>>) -> Self {
        Self {
            last_frame: Instant::now(),
            recorder,
        }
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

    async fn main(self, stream: GshStream) -> libgsh::Result<()> {
        <Self as GshServiceExt>::main(self, stream).await
    }
}

#[async_trait]
impl GshServiceExt for RdpService {
    async fn on_event(
        &mut self,
        _stream: &mut GshStream,
        event: client_message::ClientEvent,
    ) -> libgsh::Result<()> {
        log::info!("Received event: {:?}", event);
        Ok(())
    }

    async fn on_tick(&mut self, stream: &mut GshStream) -> libgsh::Result<()> {
        if self.last_frame.elapsed().as_secs_f32() >= 1.0 / MAX_FPS as f32 {
            stream.send(self.get_frame()?).await?;
            self.last_frame = std::time::Instant::now();
            log::debug!("Sent frame");
        }
        Ok(())
    }

    async fn on_startup(&mut self, stream: &mut GshStream) -> libgsh::Result<()> {
        stream.send(self.get_frame()?).await?;
        log::debug!("Sent initial frame");
        Ok(())
    }
}

impl RdpService {
    fn get_frame(&mut self) -> libgsh::Result<Frame> {
        let frame = {
            let video_stream = self.recorder.lock().unwrap();
            video_stream.recv().map_err(|e| {
                ServiceError::Error(format!("Failed to receive frame from video stream: {}", e))
            })?
        };

        log::debug!(
            "Captured image of resolution {}x{} and size: {}",
            frame.width,
            frame.height,
            frame.raw.len()
        );
        let compressed = self.compress(&frame.raw, frame.width as usize, frame.height as usize)?;
        log::debug!(
            "Compressed image size: {} (~{:.2}%)",
            compressed.len(),
            compressed.len() as f32 * 100f32 / frame.raw.len() as f32
        );
        Ok(Frame {
            window_id: WINDOW_ID,
            width: frame.width,
            height: frame.height,
            segments: full_frame_segment(
                &compressed,
                frame.width as usize,
                frame.height as usize,
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
