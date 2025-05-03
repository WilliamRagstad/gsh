pub mod r#async;
pub mod auth;
pub mod sync;

pub use prost;

pub mod protocol {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    StatusUpdate(protocol::StatusUpdate),
    UserInput(protocol::UserInput),
}

pub const PROTOCOL_VERSION: u32 = 1;

type LengthType = u32;
const LENGTH_SIZE: usize = std::mem::size_of::<LengthType>();

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    IoError(#[from] std::io::Error),
    ProstDecodeError(#[from] prost::DecodeError),
    Pkcs1Error(#[from] rsa::pkcs1::Error),
    PasswordRequired,
    InvalidPassword,
    SignatureRequired,
    SignatureInvalid,
    AnyError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::IoError(err) => write!(f, "IO error: {}", err),
            HandshakeError::Pkcs1Error(err) => write!(f, "PKCS#1 error: {}", err),
            HandshakeError::PasswordRequired => write!(f, "Password required"),
            HandshakeError::InvalidPassword => write!(f, "Invalid password"),
            HandshakeError::SignatureRequired => write!(f, "Signature required"),
            HandshakeError::SignatureInvalid => write!(f, "Signature invalid"),
            HandshakeError::ProstDecodeError(err) => write!(f, "Prost decode error: {}", err),
            HandshakeError::AnyError(err) => write!(f, "{}", err),
        }
    }
}
