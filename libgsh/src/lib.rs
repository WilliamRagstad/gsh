//! Graphical services over TLS, with a Tokio server and client protocol support.

pub use async_trait;
pub use rcgen;
pub use rsa;
pub use sha2;
pub use tokio;
pub use tokio_rustls::{self, rustls::ServerConfig};
pub use zstd;

pub mod client;
pub mod pacing;
pub mod server;
pub mod shared;
pub mod telemetry;
#[cfg(feature = "webrtc")]
pub mod webrtc;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Error(String),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error(transparent)]
    RustlsError(#[from] tokio_rustls::rustls::Error),
    #[error(transparent)]
    AnyError(#[from] Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    HandshakeError(#[from] shared::HandshakeError),
}

pub type Result<T> = std::result::Result<T, ServiceError>;
