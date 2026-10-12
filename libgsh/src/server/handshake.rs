use super::ServerStream;
use crate::shared::{
    auth::{AuthVerifier, AuthenticationError},
    protocol::{
        self, client_message::ClientEvent, server_auth_ack::AuthStatus, status_update::StatusType,
        ClientHello, ServerHelloAck,
    },
    HandshakeError,
};

/// Negotiates protocol settings and optional authentication; rejects incompatible versions.
pub async fn handshake(
    stream: &mut ServerStream,
    supported_protocol_versions: &[u32],
    server_hello: ServerHelloAck,
    auth_verifier: Option<AuthVerifier>,
) -> Result<ClientHello, HandshakeError> {
    let auth_method = server_hello.auth_method.clone();
    let ClientEvent::ClientHello(client_hello) = stream.receive().await? else {
        return Err(HandshakeError::AnyError(
            "Expected ClientHello message".into(),
        ));
    };
    if !supported_protocol_versions.contains(&client_hello.protocol_version) {
        let msg = format!(
            "Unsupported client protocol version: {}. Supported versions: {:?}",
            client_hello.protocol_version, supported_protocol_versions
        );
        stream
            .send(protocol::StatusUpdate {
                kind: StatusType::Exit as i32,
                details: None,
            })
            .await?;
        return Err(HandshakeError::AnyError(msg.into()));
    }
    stream.send(server_hello).await?;

    if auth_method.is_some() {
        let ClientEvent::ClientAuth(client_auth) = stream.receive().await? else {
            return Err(HandshakeError::AnyError(
                "Expected ClientAuth message".into(),
            ));
        };
        let result = match (auth_method.as_ref(), auth_verifier.as_ref()) {
            (Some(method), Some(verifier)) => {
                verifier.verify(method, client_auth.auth_data.as_ref())
            }
            _ => Err(AuthenticationError::MethodMismatch),
        };
        match result {
            Ok(()) => {
                stream
                    .send(protocol::ServerAuthAck {
                        status: AuthStatus::Success as i32,
                        message: "Authentication successful".to_string(),
                    })
                    .await?;
            }
            Err(error) => {
                stream
                    .send(protocol::ServerAuthAck {
                        status: AuthStatus::Failure as i32,
                        message: error.to_string(),
                    })
                    .await?;
                return Err(match error {
                    AuthenticationError::PasswordRequired => HandshakeError::PasswordRequired,
                    AuthenticationError::InvalidPassword => HandshakeError::InvalidPassword,
                    AuthenticationError::SignatureRequired => HandshakeError::SignatureRequired,
                    AuthenticationError::SignatureInvalid => HandshakeError::SignatureInvalid,
                    other => HandshakeError::AnyError(
                        std::io::Error::new(std::io::ErrorKind::PermissionDenied, other).into(),
                    ),
                });
            }
        }
    }

    Ok(client_hello)
}
