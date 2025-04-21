use shared::r#async::AsyncMessageCodec;
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;

pub mod server;
pub mod service;

/// Asynchronous message codec for the `TlsStream` over a `TcpStream`.\
pub type Messages = AsyncMessageCodec<TlsStream<TcpStream>>;
