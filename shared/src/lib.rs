use std::io::{Read, Write};

pub use prost;
use prost::Message;
use protocol::ClientHello;

pub mod protocol {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
}
pub enum ClientEvent {
    StatusUpdate(protocol::StatusUpdate),
    UserInput(protocol::UserInput),
}

pub const PROTOCOL_VERSION: u32 = 1;

type LengthType = u32;
const LENGTH_SIZE: usize = std::mem::size_of::<LengthType>();

/// A codec for reading and writing length-value encoded messages.
pub struct MessageCodec<S: Read + Write + Send> {
    /// The underlying reader and writer stream.
    stream: S,
    /// The buffer to store the read data.
    buf: Vec<u8>,
}

impl<S: Read + Write + Send> MessageCodec<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    pub fn get_stream(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Reads a whole length-value encoded message from the underlying reader.
    /// Returns the message bytes as a `Vec<u8>`.
    pub fn read_message(&mut self) -> std::io::Result<prost::bytes::Bytes> {
        let mut length_buf = [0; LENGTH_SIZE];
        self.stream.read_exact(&mut length_buf)?;
        let length = LengthType::from_be_bytes(length_buf) as usize;
        self.buf.resize(length, 0);
        self.stream.read_exact(&mut self.buf)?;
        // Convert the Vec<u8> to Bytes for better performance
        // and to avoid unnecessary allocations.
        let bytes = prost::bytes::Bytes::from(self.buf.clone());
        self.buf.clear(); // Clear the buffer for future reads
        Ok(bytes)
    }

    /// Writes a length-value encoded message to the underlying writer.
    pub fn write_message<T: Message>(&mut self, message: T) -> std::io::Result<()> {
        let message = message.encode_to_vec();
        let mut buf: Vec<u8> = Vec::with_capacity(LENGTH_SIZE + message.len());
        let length = message.len() as LengthType;
        let length_buf = length.to_be_bytes();
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
) -> std::io::Result<Option<protocol::WindowSettings>>
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
        version: PROTOCOL_VERSION,
        os,
        os_version,
    })?;

    let server_hello = protocol::ServerHelloAck::decode(messages.read_message()?)?;

    Ok(server_hello.initial_window_settings)
}

/// Handshake function for the **server side**.
/// It reads a `ClientHello` message and sends a `ServerHelloAck` response.
/// If the client version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub fn handshake_server<S>(
    messages: &mut MessageCodec<S>,
    supported_protocol_versions: &[u32],
    initial_window_settings: Option<protocol::WindowSettings>,
) -> std::io::Result<ClientHello>
where
    S: Read + Write + Send,
{
    let client_hello = protocol::ClientHello::decode(messages.read_message()?)?;

    if !supported_protocol_versions.contains(&client_hello.version) {
        let msg = format!(
            "Unsupported client protocol version: {}. Supported versions: {:?}",
            client_hello.version, supported_protocol_versions
        );
        messages.write_message(protocol::StatusUpdate {
            kind: protocol::status_update::StatusType::Exit as i32,
            message: msg.clone(),
            code: 0,
        })?;
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, msg));
    }

    messages.write_message(protocol::ServerHelloAck {
        initial_window_settings,
    })?;

    Ok(client_hello)
}
