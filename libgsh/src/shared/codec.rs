#[allow(unused_imports)]
use crate::shared::protocol::{
    client_message::ClientEvent, server_message::ServerEvent, ClientMessage, ServerMessage,
};
use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{timeout, Duration};

type LengthType = u32;
const LENGTH_SIZE: usize = std::mem::size_of::<LengthType>();
/// Hard per-message wire limit. Prevents a peer-controlled length prefix from
/// causing an unbounded allocation before protobuf parsing starts.
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

/// A codec for reading and writing length-value encoded messages.
#[derive(Debug)]
pub struct GshCodec<S: AsyncRead + AsyncWrite + Send + Unpin> {
    stream: S,
    buf: Vec<u8>,
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

    /// Reads a complete length-value encoded message.
    pub(crate) async fn read_internal(&mut self) -> std::io::Result<prost::bytes::Bytes> {
        let read_timeout = Duration::from_millis(10);

        if !self.partial_read {
            let mut length_buf = [0; LENGTH_SIZE];
            timeout(read_timeout, self.stream.read_exact(&mut length_buf)).await??;
            self.length = LengthType::from_be_bytes(length_buf) as usize;
            if self.length > MAX_MESSAGE_SIZE {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("message exceeds {MAX_MESSAGE_SIZE}-byte limit"),
                ));
            }
            self.buf.resize(self.length, 0);
        }
        self.partial_read = true;
        timeout(read_timeout, self.stream.read_exact(&mut self.buf)).await??;
        let bytes = prost::bytes::Bytes::from(std::mem::replace(
            &mut self.buf,
            Vec::with_capacity(self.length),
        ));
        self.partial_read = false;
        Ok(bytes)
    }

    /// Writes a length-value encoded message to the underlying writer.
    #[inline]
    pub(crate) async fn write_internal<T: Message>(&mut self, message: T) -> std::io::Result<()> {
        let length = message.encoded_len();
        if length > MAX_MESSAGE_SIZE || length > LengthType::MAX as usize {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("message exceeds {MAX_MESSAGE_SIZE}-byte limit"),
            ));
        }
        let mut buf = Vec::with_capacity(LENGTH_SIZE + length);
        let length_buf = (length as LengthType).to_be_bytes();
        assert_eq!(length_buf.len(), LENGTH_SIZE);
        buf.extend_from_slice(&length_buf);
        message
            .encode(&mut buf)
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        self.stream.write_all(&buf).await?;
        // Callers flush once per batch/frame to avoid per-message syscall overhead.
        Ok(())
    }

    /// Explicitly flush the underlying stream. Use this after sending a batch/frame.
    pub async fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn rejects_oversized_length_before_allocating_payload() {
        let (mut writer, reader) = duplex(8);
        writer
            .write_all(&((MAX_MESSAGE_SIZE as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        let mut codec = GshCodec::new(reader);
        let error = codec.read_internal().await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn writes_single_length_prefixed_protobuf_payload() {
        let (writer, reader) = duplex(64);
        let mut encoder = GshCodec::new(writer);
        let mut decoder = GshCodec::new(reader);
        let message = crate::shared::protocol::StatusUpdate::default();
        encoder.write_internal(message.clone()).await.unwrap();
        let bytes = decoder.read_internal().await.unwrap();
        let decoded = crate::shared::protocol::StatusUpdate::decode(bytes).unwrap();
        assert_eq!(decoded, message);
    }
}
