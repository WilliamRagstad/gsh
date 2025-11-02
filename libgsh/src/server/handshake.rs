use super::GshStream;
use crate::shared::{
    auth::AuthVerifier,
    protocol::{
        self, client_auth::AuthData, client_message::ClientEvent, server_auth_ack::AuthStatus,
        server_hello_ack::AuthMethod, status_update::StatusType, ClientHello, ServerHelloAck,
    },
    HandshakeError,
};
use rsa::RsaPublicKey;
use rsa::{pkcs1::DecodeRsaPublicKey, pkcs1v15::Signature};
use rsa::{pkcs1v15::VerifyingKey, signature::Verifier};
use sha2::Sha256;

/// Handshake function for the **server side**.
/// It reads a `ClientHello` message and sends a `ServerHelloAck` response.
/// If the client version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub async fn handshake(
    stream: &mut GshStream,
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

    // Verify ClientAuth message if auth_method is set
    if let Some(AuthMethod::Password(_)) = auth_method {
        let ClientEvent::ClientAuth(client_auth) = stream.receive().await? else {
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
            stream
                .send(protocol::ServerAuthAck {
                    status: AuthStatus::Failure as i32,
                    message: "Password is required".to_string(),
                })
                .await?;
            return Err(HandshakeError::PasswordRequired);
        }
        if !password_verifier.verify(&client_auth.password) {
            stream
                .send(protocol::ServerAuthAck {
                    status: AuthStatus::Failure as i32,
                    message: "Invalid password".to_string(),
                })
                .await?;
            return Err(HandshakeError::InvalidPassword);
        } else {
            stream
                .send(protocol::ServerAuthAck {
                    status: AuthStatus::Success as i32,
                    message: "Password verified".to_string(),
                })
                .await?;
        }
    } else if let Some(AuthMethod::Signature(server_auth)) = auth_method {
        let ClientEvent::ClientAuth(client_auth) = stream.receive().await? else {
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
            stream
                .send(protocol::ServerAuthAck {
                    status: AuthStatus::Failure as i32,
                    message: "Signature is required".to_string(),
                })
                .await?;
            return Err(HandshakeError::SignatureRequired);
        }
        let public_key_pem = String::from_utf8_lossy(&client_auth.public_key);
        let public_key = match RsaPublicKey::from_pkcs1_pem(&public_key_pem) {
            Ok(public_key) => public_key,
            Err(err) => {
                stream
                    .send(protocol::ServerAuthAck {
                        status: AuthStatus::Failure as i32,
                        message: format!("Invalid public key: {}", err),
                    })
                    .await?;
                return Err(HandshakeError::SignatureInvalid);
            }
        };
        let signature = match Signature::try_from(&client_auth.signature[..]) {
            Ok(signature) => signature,
            Err(err) => {
                stream
                    .send(protocol::ServerAuthAck {
                        status: AuthStatus::Failure as i32,
                        message: format!("Invalid signature: {}", err),
                    })
                    .await?;
                return Err(HandshakeError::SignatureInvalid);
            }
        };

        if !signature_verifier.verify(&public_key) {
            stream
                .send(protocol::ServerAuthAck {
                    status: AuthStatus::Failure as i32,
                    message: "Verification failed".to_string(),
                })
                .await?;
            return Err(HandshakeError::SignatureInvalid);
        }
        if !verify_signature(&server_auth.sign_message, signature, public_key) {
            stream
                .send(protocol::ServerAuthAck {
                    status: AuthStatus::Failure as i32,
                    message: "Verification failed".to_string(),
                })
                .await?;
            return Err(HandshakeError::SignatureInvalid);
        }
        stream
            .send(protocol::ServerAuthAck {
                status: AuthStatus::Success as i32,
                message: "Signature verified!".to_string(),
            })
            .await?;
    }

    Ok(client_hello)
}

/// Verify the signature using the public key and the sign message from the server
fn verify_signature(sign_message: &[u8], signature: Signature, public_key: RsaPublicKey) -> bool {
    let verifying_key = VerifyingKey::<Sha256>::new(public_key);
    verifying_key.verify(sign_message, &signature).is_ok()
}
