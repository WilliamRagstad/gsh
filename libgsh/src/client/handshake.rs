use super::ClientStream;
use crate::shared::{
    auth::AuthProvider,
    protocol::{
        self, client_auth,
        client_hello::MonitorInfo,
        server_auth_ack::AuthStatus,
        server_hello_ack::{AuthMethod, SignatureMethod},
        ServerHelloAck,
    },
    HandshakeError, PROTOCOL_VERSION,
};
use rsa::pkcs1v15::Signature;
use rsa::signature::SignatureEncoding;
use rsa::{pkcs1::EncodeRsaPublicKey, RsaPublicKey};

/// Handshake function for the **client side**.
/// It sends a `ClientHello` message and waits for a `ServerHelloAck` response.
/// If the server version is not compatible, it sends a `StatusUpdate` message and returns an error.
pub async fn handshake<A>(
    stream: &mut ClientStream,
    monitors: Vec<MonitorInfo>,
    mut auth_provider: A,
    host: &str,
) -> Result<ServerHelloAck, HandshakeError>
where
    A: AuthProvider,
{
    use crate::shared::protocol::server_message::ServerEvent;

    let os = match std::env::consts::OS {
        "linux" => protocol::client_hello::Os::Linux,
        "windows" => protocol::client_hello::Os::Windows,
        "macos" => protocol::client_hello::Os::Macos,
        _ => protocol::client_hello::Os::Unknown,
    } as i32;
    let os_version = os_info::get().version().to_string();
    stream
        .send(protocol::ClientHello {
            protocol_version: PROTOCOL_VERSION,
            os,
            os_version,
            monitors,
        })
        .await?;
    let ServerEvent::ServerHelloAck(server_hello) = stream.receive().await? else {
        return Err(HandshakeError::AnyError(
            "Expected ServerHelloAck message".into(),
        ));
    };

    // Send ClientAuth message if auth_method is set
    if let Some(AuthMethod::Password(_)) = server_hello.auth_method {
        stream
            .send(protocol::ClientAuth {
                auth_data: Some(client_auth::AuthData::Password(client_auth::Password {
                    password: auth_provider.password(host),
                })),
            })
            .await?;
        // Wait for ServerAuthAck message
        let ServerEvent::ServerAuthAck(server_auth_ack) = stream.receive().await? else {
            return Err(HandshakeError::AnyError(
                "Expected ServerAuthAck message".into(),
            ));
        };
        if server_auth_ack.status != AuthStatus::Success as i32 {
            return Err(HandshakeError::InvalidPassword);
        }
        auth_provider.password_success_cb();
    } else if let Some(AuthMethod::Signature(SignatureMethod { sign_message })) =
        &server_hello.auth_method
    {
        let (signature, public_key): (Signature, RsaPublicKey) = auth_provider
            .signature(host, sign_message)
            .ok_or(HandshakeError::SignatureRequired)?;
        let public_key_pem = public_key.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)?;
        let public_key_pem_bytes = public_key_pem.as_bytes().to_vec();
        let signature_bytes = signature.to_bytes().to_vec();
        stream
            .send(protocol::ClientAuth {
                auth_data: Some(client_auth::AuthData::Signature(client_auth::Signature {
                    signature: signature_bytes,
                    public_key: public_key_pem_bytes,
                })),
            })
            .await?;
        // Wait for ServerAuthAck message
        let ServerEvent::ServerAuthAck(server_auth_ack) = stream.receive().await? else {
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
