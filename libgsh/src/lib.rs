//! # Lib Graphical Shell (libgsh)
//!
//! This library provides a framework for creating graphical shell applications using Rust.
//! It includes support for both synchronous and asynchronous services, as well as TLS support using Rustls.
//! It also provides a simple server implementation for handling client connections and managing the application service.

pub use async_trait;
pub use rcgen;
pub use rsa;
pub use sha2;
pub use tokio;
pub use tokio_rustls;
pub use zstd;

#[cfg(not(feature = "client"))]
pub mod r#async;
pub mod cert;
pub mod frame;
pub mod shared;
#[cfg(not(feature = "client"))]
pub mod simple;

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
