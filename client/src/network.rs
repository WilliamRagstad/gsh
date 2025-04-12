use anyhow::Result;
use rustls::{
    client::danger::{ServerCertVerified, ServerCertVerifier},
    crypto::{ring as provider, CryptoProvider},
    time_provider, ClientConnection, StreamOwned,
};
use shared::{
    protocol::{self, WindowSettings},
    MessageCodec,
};
use std::{net::TcpStream, sync::Arc};

pub type Messages = MessageCodec<StreamOwned<ClientConnection, TcpStream>>;

pub fn shutdown_tls(mut messages: Messages) -> Result<()> {
    log::trace!("Exiting gracefully...");
    messages.get_stream().conn.send_close_notify();
    messages.write_message(protocol::StatusUpdate {
        kind: protocol::status_update::StatusType::Exit as i32,
        message: "Goodbye".to_string(),
        code: 0,
    })?;
    messages
        .get_stream()
        .sock
        .shutdown(std::net::Shutdown::Both)?;
    log::trace!("Connection closed.");
    drop(messages);
    Ok(())
}

pub fn connect_tls(
    host: &str,
    port: u16,
    insecure: bool,
) -> Result<(Option<WindowSettings>, Messages)> {
    let server_name = host.to_string().try_into()?;
    let tls_config = tls_config(insecure)?;
    let mut conn = rustls::ClientConnection::new(Arc::new(tls_config), server_name)?;
    let mut sock = TcpStream::connect(format!("{}:{}", host, port))?;
    conn.complete_io(&mut sock)?; // Complete the handshake with the stream
    let tls_stream = rustls::StreamOwned::new(conn, sock);

    // Check if the handshake was successful
    if tls_stream.conn.is_handshaking() {
        return Err(anyhow::anyhow!("Handshake failed"));
    }
    let mut messages = Messages::new(tls_stream);
    let initial_window_settings = shared::handshake_client(&mut messages)?;
    Ok((initial_window_settings, messages))
}

fn tls_config(insecure: bool) -> Result<rustls::ClientConfig> {
    let root_store = if insecure {
        rustls::RootCertStore::empty()
    } else {
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
    };
    let mut config = rustls::ClientConfig::builder_with_details(
        CryptoProvider {
            cipher_suites: vec![provider::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256],
            kx_groups: vec![provider::kx_group::X25519],
            ..provider::default_provider()
        }
        .into(),
        Arc::new(time_provider::DefaultTimeProvider),
    )
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_root_certificates(root_store)
    .with_no_client_auth();
    if insecure {
        config
            .dangerous()
            .set_certificate_verifier(Arc::new(NoCertificateVerification {}));
    }
    Ok(config)
}

#[derive(Debug, Clone)]
struct NoCertificateVerification {}

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
        ]
    }

    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Always return a valid certificate verification result
        Ok(ServerCertVerified::assertion())
    }
}
