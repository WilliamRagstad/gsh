use crate::shared::codec::GshCodec;
use crate::shared::protocol::{server_message::ServerEvent, ClientMessage, ServerMessage};
use prost::Message;
use std::io::Result;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

mod handshake;
pub use handshake::handshake;

/// Asynchronous message codec for the client `TlsStream` over a `TcpStream`.\
pub type ClientStream = GshCodec<TlsStream<TcpStream>>;

impl ClientStream {
    pub async fn send(&mut self, message: impl Into<ClientMessage>) -> Result<()> {
        self.write_internal(message.into()).await
    }

    pub async fn receive(&mut self) -> Result<ServerEvent> {
        Ok(ServerMessage::decode(self.read_internal().await?)?
            .server_event
            .expect("ServerEvent is required"))
    }
}
