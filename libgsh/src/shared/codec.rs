#[allow(unused_imports)]
use crate::shared::protocol::{
    client_message::ClientEvent, server_message::ServerEvent, ClientMessage, ServerMessage,
};
use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{timeout, Duration};

type LengthType = u32;
const LENGTH_SIZE: usize = std::mem::size_of::<LengthType>();

/// A codec for reading and writing length-value encoded messages.
#[derive(Debug)]
pub struct GshCodec<S: AsyncRead + AsyncWrite + Send + Unpin> {
    /// The underlying reader and writer stream.
    stream: S,
    /// The buffer to store the read data.
    buf: Vec<u8>,
    /// The length of the message to be read.
    length: usize,
    partial_read: bool,
}

impl<S: AsyncRead + AsyncWrite + Send + Unpin> GshCodec<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            length: 0,
            partial_read: false,
        }
    }

    pub fn get_inner(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Reads a whole length-value encoded message from the underlying reader.
    /// Returns the message bytes as a `Vec<u8>`.
    pub(crate) async fn read_internal(&mut self) -> std::io::Result<prost::bytes::Bytes> {
        let read_timeout = Duration::from_millis(10); // Set a 10-second timeout

        if !self.partial_read {
            let mut length_buf = [0; LENGTH_SIZE];
            timeout(read_timeout, self.stream.read_exact(&mut length_buf)).await??;
            self.length = LengthType::from_be_bytes(length_buf) as usize;
            self.buf.resize(self.length, 0);
        }
        self.partial_read = true;
        timeout(read_timeout, self.stream.read_exact(&mut self.buf)).await??;
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
    #[inline]
    pub(crate) async fn write_internal<T: Message>(&mut self, message: T) -> std::io::Result<()> {
        let message: Vec<u8> = message.encode_to_vec();
        let mut buf: Vec<u8> = Vec::new(); // with_capacity(LENGTH_SIZE + message.len());
        let length = message.len() as LengthType;
        let length_buf = length.to_be_bytes();
        assert_eq!(length_buf.len(), LENGTH_SIZE);
        buf.extend_from_slice(&length_buf);
        buf.extend_from_slice(&message);
        self.stream.write_all(&buf).await?;
        self.stream.flush().await?;
        Ok(())
    }
}
