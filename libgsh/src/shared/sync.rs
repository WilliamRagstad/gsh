use crate::shared::{
    protocol::{self, client_hello::MonitorInfo, ClientHello, ServerHelloAck},
    LengthType, LENGTH_SIZE, PROTOCOL_VERSION,
};
use prost::Message;
use std::io::{Read, Write};

use super::protocol::status_update::StatusType;

/// A codec for reading and writing length-value encoded messages.
pub struct MessageCodec<S: Read + Write + Send> {
    /// The underlying reader and writer stream.
    stream: S,
    /// The buffer to store the read data.
    length: usize,
    /// The buffer to store the read data.
    buf: Vec<u8>,

    partial_read: bool,
}

impl<S: Read + Write + Send> MessageCodec<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            length: 0,
            partial_read: false,
        }
    }

    pub fn get_stream(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Reads a whole length-value encoded message from the underlying reader.
    /// Returns the message bytes as a `Vec<u8>`.
    pub fn read_message(&mut self) -> std::io::Result<prost::bytes::Bytes> {
        if !self.partial_read {
            let mut length_buf = [0; LENGTH_SIZE];
            self.stream.read_exact(&mut length_buf)?;
            self.length = LengthType::from_be_bytes(length_buf) as usize;
            self.buf.resize(self.length, 0);
        }
        self.partial_read = true;
        self.stream.read_exact(&mut self.buf)?;
        // Convert the Vec<u8> to Bytes for better performance
        // and to avoid unnecessary allocations.
        let bytes = prost::bytes::Bytes::from(std::mem::replace(
            &mut self.buf,
            Vec::with_capacity(self.length),
        ));
        // If we managed to get here, no exception was thrown and we have a complete message.
        self.partial_read = false;
        Ok(bytes)
    }

    /// Writes a length-value encoded message to the underlying writer.
    pub fn write_message<T: Message>(&mut self, message: T) -> std::io::Result<()> {
        let message = message.encode_to_vec();
        let mut buf: Vec<u8> = Vec::new(); // with_capacity(LENGTH_SIZE + message.len());
        let length = message.len() as LengthType;
        let length_buf = length.to_be_bytes();
        assert_eq!(length_buf.len(), LENGTH_SIZE);
        buf.extend_from_slice(&length_buf);
        buf.extend_from_slice(&message);
        self.stream.write_all(&buf)?;
        self.stream.flush()?;
        Ok(())
    }
}

/// Handshake function for the **client side**.
/// It sends a `ClientHello` message and waits for a `ServerHelloAck` response.
/// If the server version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub fn handshake_client<S>(
    messages: &mut MessageCodec<S>,
    monitors: Vec<MonitorInfo>,
) -> std::io::Result<ServerHelloAck>
where
    S: Read + Write + Send,
{
    let os = match std::env::consts::OS {
        "linux" => protocol::client_hello::Os::Linux,
        "windows" => protocol::client_hello::Os::Windows,
        "macos" => protocol::client_hello::Os::Macos,
        _ => protocol::client_hello::Os::Unknown,
    } as i32;
    let os_version = os_info::get().version().to_string();
    messages.write_message(protocol::ClientHello {
        protocol_version: PROTOCOL_VERSION,
        os,
        os_version,
        monitors,
    })?;
    let server_hello = protocol::ServerHelloAck::decode(messages.read_message()?)?;
    Ok(server_hello)
}

/// Handshake function for the **server side**.
/// It reads a `ClientHello` message and sends a `ServerHelloAck` response.
/// If the client version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub fn handshake_server<S>(
    messages: &mut MessageCodec<S>,
    supported_protocol_versions: &[u32],
    server_hello: ServerHelloAck,
) -> std::io::Result<ClientHello>
where
    S: Read + Write + Send,
{
    let client_hello = protocol::ClientHello::decode(messages.read_message()?)?;
    if !supported_protocol_versions.contains(&client_hello.protocol_version) {
        let msg = format!(
            "Unsupported client protocol version: {}. Supported versions: {:?}",
            client_hello.protocol_version, supported_protocol_versions
        );
        messages.write_message(protocol::StatusUpdate {
            kind: StatusType::Exit as i32,
            details: None,
        })?;
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, msg));
    }
    messages.write_message(server_hello)?;
    Ok(client_hello)
}
