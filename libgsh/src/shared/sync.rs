#![allow(unused_imports)]

use crate::shared::{
    protocol::{
        self, client_hello::MonitorInfo, client_message::ClientEvent, server_message::ServerEvent,
        ClientHello, ServerHelloAck,
    },
    LengthType, LENGTH_SIZE, PROTOCOL_VERSION,
};
use prost::Message;
use rsa::{pkcs1::DecodeRsaPublicKey, pkcs1v15::Signature};
use rsa::{pkcs1::EncodeRsaPublicKey, RsaPublicKey};
use rsa::{
    pkcs1v15::VerifyingKey,
    signature::{SignatureEncoding, Verifier},
};
use sha2::Sha256;
use std::io::{Read, Write};

use super::{
    auth::{AuthProvider, AuthVerifier},
    protocol::{
        client_auth::{self, AuthData},
        server_auth_ack::AuthStatus,
        server_hello_ack::{self, AuthMethod, SignatureMethod},
        status_update::StatusType,
    },
    HandshakeError,
};

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
    pub fn read_message_internal(&mut self) -> std::io::Result<prost::bytes::Bytes> {
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

    /// Decode a message into the higher-level protocol Event types (synchronous).
    /// This mirrors the async version's public `read_message` helper which returns
    /// decoded `ServerEvent` / `ClientEvent`. We keep the raw `read_message` above
    /// for callers that want the raw bytes (eg. the simple service loop).
    #[cfg(feature = "client")]
    pub fn read_event(&mut self) -> std::io::Result<protocol::server_message::ServerEvent> {
        let bytes = self.read_message_internal()?;
        Ok(protocol::ServerMessage::decode(bytes)?
            .server_event
            .expect("ServerEvent is required"))
    }

    #[cfg(not(feature = "client"))]
    pub fn read_event(&mut self) -> std::io::Result<protocol::client_message::ClientEvent> {
        let bytes = self.read_message_internal()?;
        Ok(protocol::ClientMessage::decode(bytes)?
            .client_event
            .expect("ClientEvent is required"))
    }

    /// Writes a length-value encoded message to the underlying writer.
    fn write_internal<T: Message>(&mut self, message: T) -> std::io::Result<()> {
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

    #[cfg(feature = "client")]
    pub fn write_event(
        &mut self,
        message: impl Into<protocol::ClientMessage>,
    ) -> std::io::Result<()> {
        self.write_internal(message.into())
    }

    #[cfg(not(feature = "client"))]
    pub fn write_event(
        &mut self,
        message: impl Into<protocol::ServerMessage>,
    ) -> std::io::Result<()> {
        self.write_internal(message.into())
    }
}

/// Handshake function for the **client side**.
/// It sends a `ClientHello` message and waits for a `ServerHelloAck` response.
/// If the server version is not compatible, it sends a `StatusUpdate` message and returns an error.
#[cfg(feature = "client")]
pub fn handshake_client<S, A>(
    messages: &mut MessageCodec<S>,
    monitors: Vec<MonitorInfo>,
    mut auth_provider: A,
    host: &str,
) -> Result<ServerHelloAck, HandshakeError>
where
    S: Read + Write + Send,
    A: AuthProvider,
{
    let os = match std::env::consts::OS {
        "linux" => protocol::client_hello::Os::Linux,
        "windows" => protocol::client_hello::Os::Windows,
        "macos" => protocol::client_hello::Os::Macos,
        _ => protocol::client_hello::Os::Unknown,
    } as i32;
    let os_version = os_info::get().version().to_string();
    messages.write_internal(protocol::ClientHello {
        protocol_version: PROTOCOL_VERSION,
        os,
        os_version,
        monitors,
    })?;
    let ServerEvent::ServerHelloAck(server_hello) = messages.read_event()? else {
        return Err(HandshakeError::AnyError(
            "Expected ServerHelloAck message".into(),
        ));
    };

    // Send ClientAuth message if auth_method is set
    if let Some(server_hello_ack::AuthMethod::Password(_)) = server_hello.auth_method {
        messages.write_internal(protocol::ClientAuth {
            auth_data: Some(client_auth::AuthData::Password(client_auth::Password {
                password: auth_provider.password(host),
            })),
        })?;
        // Wait for ServerAuthAck message
        let ServerEvent::ServerAuthAck(server_auth_ack) = messages.read_event()? else {
            return Err(HandshakeError::AnyError(
                "Expected ServerAuthAck message".into(),
            ));
        };
        if server_auth_ack.status != AuthStatus::Success as i32 {
            return Err(HandshakeError::InvalidPassword);
        }
        auth_provider.password_success_cb();
    } else if let Some(server_hello_ack::AuthMethod::Signature(SignatureMethod { sign_message })) =
        &server_hello.auth_method
    {
        let (signature, public_key): (Signature, RsaPublicKey) = auth_provider
            .signature(host, sign_message)
            .ok_or(HandshakeError::SignatureRequired)?;
        let public_key_pem = public_key.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)?;
        let public_key_pem_bytes = public_key_pem.as_bytes().to_vec();
        let signature_bytes = signature.to_bytes().to_vec();
        messages.write_internal(protocol::ClientAuth {
            auth_data: Some(client_auth::AuthData::Signature(client_auth::Signature {
                signature: signature_bytes,
                public_key: public_key_pem_bytes,
            })),
        })?;
        // Wait for ServerAuthAck message
        let ServerEvent::ServerAuthAck(server_auth_ack) = messages.read_event()? else {
            return Err(HandshakeError::AnyError(
                "Expected ServerAuthAck message".into(),
            ));
        };
        if server_auth_ack.status != AuthStatus::Success as i32 {
            return Err(HandshakeError::SignatureInvalid);
        }
        auth_provider.signature_success_cb();
    } else if server_hello.auth_method.is_none() {
        log::debug!("No authentication method required by the server.");
    } else {
        return Err(HandshakeError::AnyError(
            "Unsupported authentication method".into(),
        ));
    }

    Ok(server_hello)
}

/// Handshake function for the **server side**.
/// It reads a `ClientHello` message and sends a `ServerHelloAck` response.
/// If the client version is not compatible, it sends a `StatusUpdate` message and returns an error.
#[cfg(not(feature = "client"))]
pub fn handshake_server<S>(
    messages: &mut MessageCodec<S>,
    supported_protocol_versions: &[u32],
    server_hello: ServerHelloAck,
    auth_verifier: Option<AuthVerifier>,
) -> Result<ClientHello, HandshakeError>
where
    S: Read + Write + Send,
{
    let auth_method = server_hello.auth_method.clone();
    let ClientEvent::ClientHello(client_hello) = messages.read_event()? else {
        return Err(HandshakeError::AnyError(
            "Expected ClientHello message".into(),
        ));
    };
    if !supported_protocol_versions.contains(&client_hello.protocol_version) {
        let msg = format!(
            "Unsupported client protocol version: {}. Supported versions: {:?}",
            client_hello.protocol_version, supported_protocol_versions
        );
        messages.write_event(protocol::StatusUpdate {
            kind: StatusType::Exit as i32,
            details: None,
        })?;
        return Err(HandshakeError::AnyError(msg.into()));
    }
    messages.write_event(server_hello)?;

    // Verify ClientAuth message if auth_method is set

    if let Some(AuthMethod::Password(_)) = auth_method {
        let ClientEvent::ClientAuth(client_auth) = messages.read_event()? else {
            return Err(HandshakeError::AnyError(
                "Expected ClientAuth message".into(),
            ));
        };
        let auth_verifier = auth_verifier.expect("AuthVerifier is required for server handshake");
        let client_auth = client_auth.auth_data.expect("ClientAuth data is required");
        let AuthVerifier::Password(password_verifier) = auth_verifier else {
            panic!("Password verifier is required for password authentication");
        };
        let AuthData::Password(client_auth) = client_auth else {
            return Err(HandshakeError::PasswordRequired);
        };
        if client_auth.password.is_empty() {
            messages.write_event(protocol::ServerAuthAck {
                status: AuthStatus::Failure as i32,
                message: "Password is required".to_string(),
            })?;
            return Err(HandshakeError::PasswordRequired);
        }
        if !password_verifier.verify(&client_auth.password) {
            messages.write_event(protocol::ServerAuthAck {
                status: AuthStatus::Failure as i32,
                message: "Invalid password".to_string(),
            })?;
            return Err(HandshakeError::InvalidPassword);
        } else {
            messages.write_event(protocol::ServerAuthAck {
                status: AuthStatus::Success as i32,
                message: "Password verified".to_string(),
            })?;
        }
    } else if let Some(AuthMethod::Signature(server_auth)) = auth_method {
        let ClientEvent::ClientAuth(client_auth) = messages.read_event()? else {
            return Err(HandshakeError::AnyError(
                "Expected ClientAuth message".into(),
            ));
        };
        let auth_verifier = auth_verifier.expect("AuthVerifier is required for server handshake");
        let client_auth = client_auth.auth_data.expect("ClientAuth data is required");
        let AuthVerifier::Signature(signature_verifier) = auth_verifier else {
            panic!("Signature verifier is required for signature authentication");
        };
        let AuthData::Signature(client_auth) = client_auth else {
            return Err(HandshakeError::SignatureRequired);
        };
        if client_auth.signature.is_empty() {
            messages.write_event(protocol::ServerAuthAck {
                status: AuthStatus::Failure as i32,
                message: "Signature is required".to_string(),
            })?;
            return Err(HandshakeError::SignatureRequired);
        }
        let public_key_pem = String::from_utf8_lossy(&client_auth.public_key);
        let public_key = match RsaPublicKey::from_pkcs1_pem(&public_key_pem) {
            Ok(public_key) => public_key,
            Err(err) => {
                messages.write_event(protocol::ServerAuthAck {
                    status: AuthStatus::Failure as i32,
                    message: format!("Invalid public key: {}", err),
                })?;
                return Err(HandshakeError::SignatureInvalid);
            }
        };
        let signature = match Signature::try_from(&client_auth.signature[..]) {
            Ok(signature) => signature,
            Err(err) => {
                messages.write_event(protocol::ServerAuthAck {
                    status: AuthStatus::Failure as i32,
                    message: format!("Invalid signature: {}", err),
                })?;
                return Err(HandshakeError::SignatureInvalid);
            }
        };

        if !signature_verifier.verify(&public_key) {
            messages.write_event(protocol::ServerAuthAck {
                status: AuthStatus::Failure as i32,
                message: "Verification failed".to_string(),
            })?;
            return Err(HandshakeError::SignatureInvalid);
        }
        if !verify_signature(&server_auth.sign_message, signature, public_key) {
            messages.write_event(protocol::ServerAuthAck {
                status: AuthStatus::Failure as i32,
                message: "Verification failed".to_string(),
            })?;
            return Err(HandshakeError::SignatureInvalid);
        }
        messages.write_event(protocol::ServerAuthAck {
            status: AuthStatus::Success as i32,
            message: "Signature verified!".to_string(),
        })?;
    }

    Ok(client_hello)
}

#[allow(dead_code)]
/// Verify the signature using the public key and the sign message from the server
fn verify_signature(sign_message: &[u8], signature: Signature, public_key: RsaPublicKey) -> bool {
    let verifying_key = VerifyingKey::<Sha256>::new(public_key);
    verifying_key.verify(sign_message, &signature).is_ok()
}
