use crate::{
    protocol::{self, client_hello::MonitorInfo, ClientHello, ServerHelloAck},
    LengthType, LENGTH_SIZE, PROTOCOL_VERSION,
};
use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// A codec for reading and writing length-value encoded messages.
#[derive(Debug)]
pub struct AsyncMessageCodec<S: AsyncRead + AsyncWrite + Send + Unpin> {
    /// The underlying reader and writer stream.
    stream: S,
    /// The buffer to store the read data.
    buf: Vec<u8>,

    partial_read: bool,
}

impl<S: AsyncRead + AsyncWrite + Send + Unpin> AsyncMessageCodec<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            partial_read: false,
        }
    }

    pub fn get_stream(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Reads a whole length-value encoded message from the underlying reader.
    /// Returns the message bytes as a `Vec<u8>`.
    pub async fn read_message(&mut self) -> std::io::Result<prost::bytes::Bytes> {
        if !self.partial_read {
            let mut length_buf = [0; LENGTH_SIZE];
            self.stream.read_exact(&mut length_buf).await?;
            let length = LengthType::from_be_bytes(length_buf) as usize;
            self.buf.resize(length, 0);
        }
        self.partial_read = true;
        self.stream.read_exact(&mut self.buf).await?;
        // Convert the Vec<u8> to Bytes for better performance
        // and to avoid unnecessary allocations.
        let bytes = prost::bytes::Bytes::from(self.buf.clone());
        self.buf.clear(); // Clear the buffer for future reads
                          // If we managed to get here, no exception was thrown and we have a complete message.
        self.partial_read = false;
        Ok(bytes)
    }

    /// Writes a length-value encoded message to the underlying writer.
    pub async fn write_message<T: Message>(&mut self, message: T) -> std::io::Result<()> {
        let message = message.encode_to_vec();
        let mut buf: Vec<u8> = Vec::new(); // with_capacity(LENGTH_SIZE + message.len());
        let length = message.len() as LengthType;
        let length_buf = length.to_be_bytes();
        buf.extend_from_slice(&length_buf);
        buf.extend_from_slice(&message);
        self.stream.write_all(&buf).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

/// Handshake function for the **client side**.
/// It sends a `ClientHello` message and waits for a `ServerHelloAck` response.
/// If the server version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub async fn handshake_client<S>(
    messages: &mut AsyncMessageCodec<S>,
    monitors: Vec<MonitorInfo>,
) -> std::io::Result<ServerHelloAck>
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let os = match std::env::consts::OS {
        "linux" => protocol::client_hello::Os::Linux,
        "windows" => protocol::client_hello::Os::Windows,
        "macos" => protocol::client_hello::Os::Macos,
        _ => protocol::client_hello::Os::Unknown,
    } as i32;
    let os_version = os_info::get().version().to_string();
    messages
        .write_message(protocol::ClientHello {
            protocol_version: PROTOCOL_VERSION,
            os,
            os_version,
            monitors,
        })
        .await?;
    let server_hello = protocol::ServerHelloAck::decode(messages.read_message().await?)?;
    Ok(server_hello)
}

/// Handshake function for the **server side**.
/// It reads a `ClientHello` message and sends a `ServerHelloAck` response.
/// If the client version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub async fn handshake_server<S>(
    messages: &mut AsyncMessageCodec<S>,
    supported_protocol_versions: &[u32],
    server_hello: ServerHelloAck,
) -> std::io::Result<ClientHello>
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let client_hello = protocol::ClientHello::decode(messages.read_message().await?)?;
    if !supported_protocol_versions.contains(&client_hello.protocol_version) {
        let msg = format!(
            "Unsupported client protocol version: {}. Supported versions: {:?}",
            client_hello.protocol_version, supported_protocol_versions
        );
        messages
            .write_message(protocol::StatusUpdate {
                kind: protocol::status_update::StatusType::Exit as i32,
                details: Some(protocol::status_update::Details::Exit(
                    protocol::status_update::Exit {},
                )),
            })
            .await?;
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, msg));
    }
    messages.write_message(server_hello).await?;
    Ok(client_hello)
}
