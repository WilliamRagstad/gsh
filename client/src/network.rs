use std::sync::Arc;

use anyhow::Result;
use libgsh::shared::{
    protocol::{
        self,
        client_hello::MonitorInfo,
        status_update::{Details, Exit, StatusType},
        ServerHelloAck,
    },
    r#async::AsyncMessageCodec,
};
use tokio::{io::AsyncWriteExt, net::TcpStream};
use tokio_rustls::rustls::{
    self,
    client::danger::{ServerCertVerified, ServerCertVerifier},
    crypto::{ring as provider, CryptoProvider},
    time_provider,
};
// use std::{net::TcpStream, sync::Arc};
use tokio_rustls::{client::TlsStream, TlsConnector};

// pub type Messages = MessageCodec<StreamOwned<ClientConnection, TcpStream>>;
pub type Messages = AsyncMessageCodec<TlsStream<TcpStream>>;

pub async fn shutdown_tls(messages: &mut Messages) -> Result<()> {
    log::trace!("Exiting gracefully...");
    messages.get_stream().get_mut().1.send_close_notify();
    messages
        .write_message(protocol::StatusUpdate {
            kind: StatusType::Exit as i32,
            details: Some(Details::Exit(Exit {})),
        })
        .await?;
    messages.get_stream().get_mut().0.shutdown().await?;
    log::trace!("Connection closed.");
    Ok(())
}

pub async fn connect_tls(
    host: &str,
    port: u16,
    insecure: bool,
    monitors: Vec<MonitorInfo>,
) -> Result<(ServerHelloAck, Messages)> {
    let server_name = host.to_string().try_into()?;
    let tls_config = Arc::new(tls_config(insecure)?);
    let tls_connector = TlsConnector::from(tls_config);
    let sock = TcpStream::connect(format!("{}:{}", host, port)).await?;
    let tls_stream = tls_connector.connect(server_name, sock).await?;
    let mut messages = Messages::new(tls_stream);
    let hello = libgsh::shared::r#async::handshake_client(&mut messages, monitors).await?;
    Ok((hello, messages))
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
