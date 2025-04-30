use std::net::TcpStream;

use crate::shared::sync::MessageCodec;
use tokio_rustls::rustls::{ServerConnection, StreamOwned};

pub mod server;
pub mod service;

/// Synchronous message codec for the `StreamOwned` over a `TcpStream`.\
pub type Messages = MessageCodec<StreamOwned<ServerConnection, TcpStream>>;
