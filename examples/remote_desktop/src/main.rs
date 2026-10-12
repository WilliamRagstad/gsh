mod application;
mod frame_source;
mod signaling;

use anyhow::Context;
use application::{RemoteDesktopApplication, RemoteDesktopAuth};
use axum_server::tls_rustls::RustlsConfig;
use frame_source::{FrameHub, FrameSource};
use libgsh::{
    async_trait::async_trait,
    server::{GshServer, GshService, GshServiceExt, ServerStream},
    shared::frame::full_frame_segment,
    shared::{
        auth::AuthVerifier,
        protocol::{client_message, Frame},
    },
    tokio,
    tokio_rustls::rustls::ServerConfig,
    ServiceError,
};
use std::{
    fs::File,
    io::{BufReader, Write},
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use xcap::Monitor;

const ZSTD_COMPRESSION_LEVEL: i32 = 3;
const WINDOW_ID: u32 = 0;
const MAX_FPS: u32 = 60;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_line_number(true)
        .format_file(true)
        .format_target(false)
        .format_timestamp(None)
        .init();

    let config = load_tls_config()?;
    let auth = configured_auth()?;
    let legacy_tcp = std::env::args().any(|arg| arg == "--legacy-tcp");
    let wss_address = if legacy_tcp {
        None
    } else {
        Some(
            std::env::var("GSH_WSS_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8787".to_string())
                .parse::<SocketAddr>()?,
        )
    };
    let allowed_origins = if legacy_tcp {
        Vec::new()
    } else {
        configured_origins()?
    };

    let monitor = Monitor::all()?
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .unwrap_or_else(|| Monitor::from_point(0, 0).expect("no monitor found"));
    let width = monitor.width()?;
    let height = monitor.height()?;
    let (video_recorder, video_stream) = monitor
        .video_recorder()
        .expect("Failed to create video recorder");
    video_recorder
        .start()
        .expect("Failed to start video recorder");

    let (frame_hub, capture_forwarder) = FrameHub::from_xcap(video_stream);

    let application = RemoteDesktopApplication::new(frame_hub, auth, width, height);
    let server_result = if legacy_tcp {
        eprintln!("Deprecated TCP/TLS transport enabled on port 1122");
        let server = GshServer::new(RdpService::new(application), (*config).clone());
        tokio::select! {
            result = server.serve() => result.map_err(anyhow::Error::from),
            signal = tokio::signal::ctrl_c() => {
                signal?;
                Ok(())
            }
        }
    } else {
        let address = wss_address.expect("WSS address loaded when legacy TCP is disabled");
        let handle = axum_server::Handle::new();
        let server = axum_server::bind_rustls(address, RustlsConfig::from_config(config.clone()))
            .handle(handle.clone())
            .serve(
                signaling::router(application, allowed_origins)
                    .into_make_service_with_connect_info::<SocketAddr>(),
            );
        eprintln!("GSH WebRTC remote desktop listening at wss://{address}/");
        eprintln!("Install the issuing private CA in browser and native-client trust stores.");
        tokio::pin!(server);
        tokio::select! {
            result = &mut server => result.map_err(anyhow::Error::from),
            signal = tokio::signal::ctrl_c() => {
                signal?;
                handle.graceful_shutdown(Some(Duration::from_secs(5)));
                (&mut server).await.map_err(anyhow::Error::from)
            }
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
    server_result?;
    Ok(())
}

fn load_tls_config() -> anyhow::Result<Arc<ServerConfig>> {
    let _ = libgsh::tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let certificate_path = std::env::var("GSH_TLS_CERT_FILE")?;
    let private_key_path = std::env::var("GSH_TLS_KEY_FILE")?;
    let certificates = rustls_pemfile::certs(&mut BufReader::new(File::open(&certificate_path)?))
        .collect::<Result<Vec<_>, _>>()
        .context("failed to parse GSH_TLS_CERT_FILE")?;
    let private_key =
        rustls_pemfile::private_key(&mut BufReader::new(File::open(&private_key_path)?))
            .context("failed to parse GSH_TLS_KEY_FILE")?
            .ok_or_else(|| anyhow::anyhow!("GSH_TLS_KEY_FILE contains no private key"))?;
    Ok(Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certificates, private_key)
            .context("TLS certificate and private key do not match")?,
    ))
}

fn configured_auth() -> anyhow::Result<RemoteDesktopAuth> {
    let password_file = std::env::var_os("GSH_PASSWORD_FILE");
    let authorized_keys_file = std::env::var_os("GSH_AUTHORIZED_KEYS_FILE");
    match (password_file, authorized_keys_file) {
        (Some(path), None) => {
            let password = std::fs::read_to_string(path)?
                .trim_end_matches(&['\r', '\n'][..])
                .to_owned();
            anyhow::ensure!(!password.is_empty(), "GSH_PASSWORD_FILE must not be empty");
            Ok(RemoteDesktopAuth::password(password))
        }
        (None, Some(path)) => {
            let keys = std::fs::read_to_string(path)?;
            RemoteDesktopAuth::from_authorized_keys_pem(&keys).map_err(anyhow::Error::msg)
        }
        _ => anyhow::bail!("set exactly one of GSH_PASSWORD_FILE or GSH_AUTHORIZED_KEYS_FILE"),
    }
}

fn configured_origins() -> anyhow::Result<Vec<String>> {
    let value = std::env::var("GSH_ALLOWED_ORIGINS")?;
    let origins = value
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !origins.is_empty(),
        "GSH_ALLOWED_ORIGINS must contain at least one https:// origin"
    );
    anyhow::ensure!(
        origins.iter().all(|origin| {
            origin.strip_prefix("https://").is_some_and(|authority| {
                !authority.is_empty()
                    && authority.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || matches!(character, '.' | '-' | ':' | '[' | ']')
                    })
            })
        }),
        "GSH_ALLOWED_ORIGINS must contain comma-separated https:// DNS/IPv4/IPv6 origins without paths"
    );
    Ok(origins)
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
        self.application.server_hello()
    }

    fn auth_verifier(&self) -> Option<AuthVerifier> {
        Some(self.application.auth_verifier())
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
