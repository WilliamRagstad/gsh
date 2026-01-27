//! # Lib Graphical Shell (libgsh)
//!
//! This library provides a framework for creating graphical shell applications using Rust.
//! It includes support for both synchronous and asynchronous services, as well as TLS support using Rustls.
//! It also provides a simple server implementation for handling client connections and managing the application service.

// #[cfg(all(feature = "client", feature = "server"))]
// compile_error!("Features 'client' and 'server' cannot be enabled at the same time.");

pub use async_trait;
pub use rcgen;
pub use rsa;
pub use sha2;
pub use tokio;
pub use tokio_rustls::{self, rustls::ServerConfig};
pub use zstd;

pub mod client;
pub mod server;
pub mod shared;

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
