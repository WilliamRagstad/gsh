use crate::shared::codec::GshCodec;
use crate::shared::protocol::{client_message::ClientEvent, ClientMessage, ServerMessage};
use prost::Message;
use std::io::Result;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;

pub mod server;
pub mod service;

mod handshake;
pub use handshake::handshake;
pub use server::GshServer;
pub use service::{GshService, GshServiceExt};

/// Asynchronous message codec for the server `TlsStream` over a `TcpStream`.\
pub type GshStream = GshCodec<TlsStream<TcpStream>>;

impl<S: AsyncRead + AsyncWrite + Send + Unpin> GshCodec<S> {
    pub async fn send(&mut self, message: impl Into<ServerMessage>) -> Result<()> {
        self.write_internal(message.into()).await
    }

    pub async fn receive(&mut self) -> Result<ClientEvent> {
        Ok(ClientMessage::decode(self.read_internal().await?)?
            .client_event
            .expect("ClientEvent is required"))
    }
}
