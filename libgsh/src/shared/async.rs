use crate::shared::{
    protocol::{self, client_hello::MonitorInfo, ClientHello, ServerHelloAck},
    LengthType, LENGTH_SIZE, PROTOCOL_VERSION,
};
use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{timeout, Duration};

use super::{
    auth::{AuthProvider, AuthVerifier},
    protocol::{
        server_auth_ack::AuthStatus, server_hello_ack::AuthMethod, status_update::StatusType,
    },
    HandshakeError,
};

/// A codec for reading and writing length-value encoded messages.
#[derive(Debug)]
pub struct AsyncMessageCodec<S: AsyncRead + AsyncWrite + Send + Unpin> {
    /// The underlying reader and writer stream.
    stream: S,
    /// The buffer to store the read data.
    buf: Vec<u8>,
    /// The length of the message to be read.
    length: usize,
    partial_read: bool,
}

impl<S: AsyncRead + AsyncWrite + Send + Unpin> AsyncMessageCodec<S> {
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
    pub async fn read_message(&mut self) -> std::io::Result<prost::bytes::Bytes> {
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
    pub async fn write_message<T: Message>(&mut self, message: T) -> std::io::Result<()> {
        let message = message.encode_to_vec();
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

/// Handshake function for the **client side**.
/// It sends a `ClientHello` message and waits for a `ServerHelloAck` response.
/// If the server version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub async fn handshake_client<S, A>(
    messages: &mut AsyncMessageCodec<S>,
    monitors: Vec<MonitorInfo>,
    mut auth_provider: A,
    host: &str,
) -> Result<ServerHelloAck, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
    A: AuthProvider,
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

    // Send ClientAuth message if auth_method is set
    if server_hello.auth_method == AuthMethod::Password as i32 {
        messages
            .write_message(protocol::ClientAuth {
                password: Some(auth_provider.password(host)),
                signature: None,
            })
            .await?;
    } else if server_hello.auth_method == AuthMethod::Signature as i32 {
        messages
            .write_message(protocol::ClientAuth {
                password: None,
                signature: Some(auth_provider.signature(host)),
            })
            .await?;
    }

    Ok(server_hello)
}

/// Handshake function for the **server side**.
/// It reads a `ClientHello` message and sends a `ServerHelloAck` response.
/// If the client version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub async fn handshake_server<S>(
    messages: &mut AsyncMessageCodec<S>,
    supported_protocol_versions: &[u32],
    server_hello: ServerHelloAck,
    auth_verifier: Option<AuthVerifier>,
) -> Result<ClientHello, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let auth_method = server_hello.auth_method;
    let client_hello = protocol::ClientHello::decode(messages.read_message().await?)?;
    if !supported_protocol_versions.contains(&client_hello.protocol_version) {
        let msg = format!(
            "Unsupported client protocol version: {}. Supported versions: {:?}",
            client_hello.protocol_version, supported_protocol_versions
        );
        messages
            .write_message(protocol::StatusUpdate {
                kind: StatusType::Exit as i32,
                details: None,
            })
            .await?;
        return Err(HandshakeError::AnyError(msg.into()));
    }
    messages.write_message(server_hello).await?;

    // Verify ClientAuth message if auth_method is set
    if auth_method != AuthMethod::None as i32 {
        let client_auth = protocol::ClientAuth::decode(messages.read_message().await?)?;
        let auth_verifier = auth_verifier.expect("AuthVerifier is required for server handshake");
        if auth_method == AuthMethod::Password as i32 {
            let AuthVerifier::Password(password_verifier) = auth_verifier else {
                panic!("Password verifier is required for password authentication");
            };
            match client_auth.password {
                Some(ref password) if password.is_empty() => {
                    messages
                        .write_message(protocol::ServerAuthAck {
                            status: AuthStatus::Failure as i32,
                            message: "Password is required".to_string(),
                        })
                        .await?;
                    return Err(HandshakeError::PasswordRequired);
                }
                Some(ref password) => {
                    if !password_verifier.verify_password(password) {
                        messages
                            .write_message(protocol::ServerAuthAck {
                                status: AuthStatus::Failure as i32,
                                message: "Invalid password".to_string(),
                            })
                            .await?;
                        return Err(HandshakeError::InvalidPassword);
                    } else {
                        messages
                            .write_message(protocol::ServerAuthAck {
                                status: AuthStatus::Success as i32,
                                message: "Password verified".to_string(),
                            })
                            .await?;
                    }
                }
                None => {
                    messages
                        .write_message(protocol::ServerAuthAck {
                            status: AuthStatus::Failure as i32,
                            message: "Password is required".to_string(),
                        })
                        .await?;
                    return Err(HandshakeError::PasswordRequired);
                }
            };
        } else if auth_method == AuthMethod::Signature as i32 {
            let AuthVerifier::Signature(signature_verifier) = auth_verifier else {
                panic!("Signature verifier is required for signature authentication");
            };
            match client_auth.signature {
                Some(ref signature) if signature.is_empty() => {
                    messages
                        .write_message(protocol::ServerAuthAck {
                            status: AuthStatus::Failure as i32,
                            message: "Signature is required".to_string(),
                        })
                        .await?;
                    return Err(HandshakeError::SignatureRequired);
                }
                Some(ref signature) => {
                    if !signature_verifier.verify_signature(signature) {
                        messages
                            .write_message(protocol::ServerAuthAck {
                                status: AuthStatus::Failure as i32,
                                message: "Invalid signature".to_string(),
                            })
                            .await?;
                        return Err(HandshakeError::SignatureInvalid);
                    } else {
                        messages
                            .write_message(protocol::ServerAuthAck {
                                status: AuthStatus::Success as i32,
                                message: "Signature verified".to_string(),
                            })
                            .await?;
                    }
                }
                None => {
                    messages
                        .write_message(protocol::ServerAuthAck {
                            status: AuthStatus::Failure as i32,
                            message: "Signature is required".to_string(),
                        })
                        .await?;
                    return Err(HandshakeError::SignatureRequired);
                }
            };
        }
    }

    Ok(client_hello)
}
