pub mod r#async;
pub mod cert;
pub mod frame;
pub mod simple;

pub use rcgen;
pub use shared;
pub use tokio;
pub use tokio_rustls as rustls;

#[derive(Debug, thiserror::Error)]
pub enum SerivceError {
    IoError(#[from] std::io::Error),
    RustlsError(#[from] tokio_rustls::rustls::Error),
    AnyError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for SerivceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SerivceError::IoError(err) => write!(f, "IO error: {}", err),
            SerivceError::RustlsError(err) => write!(f, "Rustls error: {}", err),
            SerivceError::AnyError(err) => write!(f, "{}", err),
        }
    }
}

pub type Result<T> = std::result::Result<T, SerivceError>;
