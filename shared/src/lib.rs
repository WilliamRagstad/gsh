use std::io::{Read, Write};

pub use prost;

pub mod protocol {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
}

type LengthType = u16;
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
    pub fn read_message(&mut self) -> std::io::Result<Vec<u8>> {
        let mut length_buf = [0; LENGTH_SIZE];
        self.stream.read_exact(&mut length_buf)?;
        println!("Received length: {:?}", &length_buf[..]);
        let length = LengthType::from_be_bytes(length_buf) as usize;
        self.buf.resize(length, 0);
        self.stream.read_exact(&mut self.buf)?;
        println!("Received data: {:?}", &self.buf[..]);
        Ok(self.buf.clone())
    }

    /// Writes a length-value encoded message to the underlying writer.
    pub fn write_message(&mut self, message: &[u8]) -> std::io::Result<()> {
        let mut buf: Vec<u8> = Vec::with_capacity(LENGTH_SIZE + message.len());
        let length = message.len() as LengthType;
        let length_buf = length.to_be_bytes();
        buf.extend_from_slice(&length_buf);
        buf.extend_from_slice(message);
        self.stream.write_all(&buf)?;
        self.stream.flush()?;
        println!("Sent message: {:?}", &buf[..]);
        Ok(())
    }
}
