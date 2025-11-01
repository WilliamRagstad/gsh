pub mod r#async;
pub mod auth;
pub mod sync;

pub use prost;

pub mod protocol {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
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

impl From<protocol::ClientHello> for protocol::ClientMessage {
    fn from(value: protocol::ClientHello) -> Self {
        protocol::ClientMessage {
            client_event: Some(protocol::client_message::ClientEvent::ClientHello(value)),
        }
    }
}

impl From<protocol::ClientAuth> for protocol::ClientMessage {
    fn from(value: protocol::ClientAuth) -> Self {
        protocol::ClientMessage {
            client_event: Some(protocol::client_message::ClientEvent::ClientAuth(value)),
        }
    }
}

impl From<protocol::StatusUpdate> for protocol::ClientMessage {
    fn from(value: protocol::StatusUpdate) -> Self {
        protocol::ClientMessage {
            client_event: Some(protocol::client_message::ClientEvent::StatusUpdate(value)),
        }
    }
}

impl From<protocol::UserInput> for protocol::ClientMessage {
    fn from(value: protocol::UserInput) -> Self {
        protocol::ClientMessage {
            client_event: Some(protocol::client_message::ClientEvent::UserInput(value)),
        }
    }
}

impl From<protocol::ServerHelloAck> for protocol::ServerMessage {
    fn from(value: protocol::ServerHelloAck) -> Self {
        protocol::ServerMessage {
            server_event: Some(protocol::server_message::ServerEvent::ServerHelloAck(value)),
        }
    }
}

impl From<protocol::ServerAuthAck> for protocol::ServerMessage {
    fn from(value: protocol::ServerAuthAck) -> Self {
        protocol::ServerMessage {
            server_event: Some(protocol::server_message::ServerEvent::ServerAuthAck(value)),
        }
    }
}

impl From<protocol::StatusUpdate> for protocol::ServerMessage {
    fn from(value: protocol::StatusUpdate) -> Self {
        protocol::ServerMessage {
            server_event: Some(protocol::server_message::ServerEvent::StatusUpdate(value)),
        }
    }
}

impl From<protocol::Frame> for protocol::ServerMessage {
    fn from(value: protocol::Frame) -> Self {
        protocol::ServerMessage {
            server_event: Some(protocol::server_message::ServerEvent::Frame(value)),
        }
    }
}
