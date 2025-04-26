pub mod r#async;
pub mod cert;
pub mod frame;
pub mod simple;

pub use async_trait;
pub use rcgen;
pub use shared;
pub use tokio;
pub use tokio_rustls;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    IoError(#[from] std::io::Error),
    RustlsError(#[from] tokio_rustls::rustls::Error),
    AnyError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::IoError(err) => write!(f, "IO error: {}", err),
            ServiceError::RustlsError(err) => write!(f, "Rustls error: {}", err),
            ServiceError::AnyError(err) => write!(f, "{}", err),
        }
    }
}

pub type Result<T> = std::result::Result<T, ServiceError>;
