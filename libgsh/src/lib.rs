//! # Lib Graphical Shell (libgsh)
//!
//! This library provides a framework for creating graphical shell applications using Rust.
//! It includes support for both synchronous and asynchronous services, as well as TLS support using Rustls.
//! It also provides a simple server implementation for handling client connections and managing the application service.

pub mod r#async;
pub mod cert;
pub mod frame;
pub mod shared;
pub mod simple;

pub use async_trait;
pub use rcgen;
pub use rsa;
pub use sha2;
pub use tokio;
pub use tokio_rustls;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    IoError(#[from] std::io::Error),
    RustlsError(#[from] tokio_rustls::rustls::Error),
    AnyError(#[from] Box<dyn std::error::Error + Send + Sync>),
    HandshakeError(#[from] shared::HandshakeError),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::IoError(err) => write!(f, "IO error: {}", err),
            ServiceError::RustlsError(err) => write!(f, "Rustls error: {}", err),
            ServiceError::AnyError(err) => write!(f, "{}", err),
            ServiceError::HandshakeError(err) => write!(f, "Handshake error: {}", err),
        }
    }
}

pub type Result<T> = std::result::Result<T, ServiceError>;
