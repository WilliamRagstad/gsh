use std::io::{Read, Write};

pub use prost;
use prost::Message;

pub mod protocol {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
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
        println!("Received length: {:?}", &length_buf[..]);
        let length = LengthType::from_be_bytes(length_buf) as usize;
        self.buf.resize(length, 0);
        self.stream.read_exact(&mut self.buf)?;
        println!("Received data: {:?}", &self.buf[..]);
        // Ok(self.buf.clone())
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
        println!("Sent message: {:?}", &buf[..]);
        Ok(())
    }
}

/// Handshake function for the **client side**.
/// It sends a `ClientHello` message and waits for a `ServerHelloAck` response.
/// If the server version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub fn handshake_client<S>(messages: &mut MessageCodec<S>) -> std::io::Result<()>
where
    S: Read + Write + Send,
{
    messages.write_message(protocol::ClientHello {
        version: PROTOCOL_VERSION,
    })?;

    let server_hello = protocol::ServerHelloAck::decode(messages.read_message()?)?;
    println!("ServerHello: {:?}", server_hello);

    if server_hello.version != PROTOCOL_VERSION {
        messages.write_message(protocol::StatusUpdate {
            status: protocol::status_update::StatusType::Close as i32,
            message: "Invalid server version".to_string(),
            code: 0,
        })?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid server version",
        ));
    }

    Ok(())
}

/// Handshake function for the **server side**.
/// It reads a `ClientHello` message and sends a `ServerHelloAck` response.
/// If the client version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub fn handshake_server<S>(messages: &mut MessageCodec<S>) -> std::io::Result<()>
where
    S: Read + Write + Send,
{
    let client_hello = protocol::ClientHello::decode(messages.read_message()?)?;
    println!("ClientHello: {:?}", client_hello);

    if client_hello.version != PROTOCOL_VERSION {
        messages.write_message(protocol::StatusUpdate {
            status: protocol::status_update::StatusType::Close as i32,
            message: "Invalid client version".to_string(),
            code: 0,
        })?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid client version",
        ));
    }

    messages.write_message(protocol::ServerHelloAck {
        version: PROTOCOL_VERSION,
    })?;

    Ok(())
}
