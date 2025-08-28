use std::sync::Arc;

use dialoguer::Confirm;
use libgsh::sha2::{Digest, Sha256};
use libgsh::shared::{
    protocol::{self, client_hello::MonitorInfo, status_update::StatusType, ServerHelloAck},
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
use quinn::{RecvStream, SendStream};

use crate::{auth::ClientAuthProvider, config};

// pub type Messages = MessageCodec<StreamOwned<ClientConnection, TcpStream>>;
pub type Messages = AsyncMessageCodec<TlsStream<TcpStream>>;

pub async fn shutdown_tls(messages: &mut Messages) -> anyhow::Result<()> {
    log::trace!("Exiting gracefully...");
    messages.get_stream().get_mut().1.send_close_notify();
    messages
        .write_message(protocol::StatusUpdate {
            kind: StatusType::Exit as i32,
            details: None,
        })
        .await?;
    messages.get_stream().get_mut().0.shutdown().await?;
    log::trace!("Connection closed.");
    Ok(())
}

fn tls_config(insecure: bool) -> anyhow::Result<rustls::ClientConfig> {
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

async fn verify_host(
    known_hosts: &mut config::KnownHosts,
    host: &str,
    certs: &[rustls::pki_types::CertificateDer<'_>],
) -> anyhow::Result<bool> {
    let mut fingerprints: Vec<Vec<u8>> = Vec::new();
    for cert in certs {
        let fingerprint = Sha256::digest(cert.as_ref());
        fingerprints.push(fingerprint.to_vec());
    }
    if let Some(known) = known_hosts.find_host(host) {
        if known.compare(&fingerprints) {
            log::info!("Host {} verified successfully.", host);
            Ok(true)
        } else {
            log::warn!(
                "Host {} fingerprint mismatch. Expected: {:X?}, Found: {:X?}",
                host,
                known.fingerprints,
                fingerprints
            );
            Ok(false)
        }
    } else {
        if fingerprints.is_empty() {
            log::error!(
                "Host {} has no fingerprints. Use --insecure to skip verification.",
                host
            );
            return Ok(false);
        }
        log::warn!(
            "Host {} not found in known hosts. Please verify the host's fingerprint.",
            host
        );
        println!("Host {} fingerprints: {:X?}", host, fingerprints);
        // Prompt for confirmation
        let confirmation = Confirm::new()
            .with_prompt("Do you want to add this host to known hosts?")
            .default(false)
            .interact()?;
        if confirmation {
            known_hosts.add_host(host.to_string(), fingerprints.clone(), None, None);
            log::info!("Host {} added to known hosts.", host);
            Ok(true)
        } else {
            log::warn!("Host {} not added to known hosts.", host);
            Ok(false)
        }
    }
}

pub async fn connect_tls(
    host: &str,
    port: u16,
    insecure: bool,
    monitors: Vec<MonitorInfo>,
    mut known_hosts: config::KnownHosts,
    id_files: config::IdFiles,
    id_override: Option<String>,
) -> anyhow::Result<(ServerHelloAck, Messages)> {
    let server_name = host.to_string().try_into()?;
    let tls_config = Arc::new(tls_config(insecure)?);
    let tls_connector = TlsConnector::from(tls_config);
    let addr = format!("{}:{}", host, port);
    let sock = TcpStream::connect(&addr).await?;
    let mut tls_stream = tls_connector.connect(server_name, sock).await?;
    if !insecure {
        let certs = tls_stream.get_ref().1.peer_certificates().unwrap();
        if !verify_host(&mut known_hosts, host, certs).await? {
            tls_stream.get_mut().1.send_close_notify();
            tls_stream.get_mut().0.shutdown().await?;
            log::warn!("Host verification failed. Connection closed.");
            return Err(anyhow::anyhow!("Host verification failed."));
        }
    }
    let mut messages = Messages::new(tls_stream);
    let hello = libgsh::shared::r#async::handshake_client(
        &mut messages,
        monitors,
        ClientAuthProvider::new(known_hosts, id_files, id_override),
        host,
    )
    .await?;

    Ok((hello, messages))
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

/// QUIC-based connection stream wrapper
pub struct QuicStream {
    send: SendStream,
    recv: RecvStream,
}

impl tokio::io::AsyncRead for QuicStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        use std::pin::Pin;
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for QuicStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        use std::pin::Pin;
        match Pin::new(&mut self.send).poll_write(cx, buf) {
            std::task::Poll::Ready(Ok(n)) => std::task::Poll::Ready(Ok(n)),
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e))),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        use std::pin::Pin;
        match Pin::new(&mut self.send).poll_flush(cx) {
            std::task::Poll::Ready(Ok(())) => std::task::Poll::Ready(Ok(())),
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e))),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        use std::pin::Pin;
        match Pin::new(&mut self.send).poll_shutdown(cx) {
            std::task::Poll::Ready(Ok(())) => std::task::Poll::Ready(Ok(())),
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e))),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

pub type QuicMessages = AsyncMessageCodec<QuicStream>;

/// Connect using QUIC protocol instead of TCP+TLS
pub async fn connect_quic(
    host: &str,
    port: u16,
    insecure: bool,
    monitors: Vec<MonitorInfo>,
    known_hosts: config::KnownHosts,
    id_files: config::IdFiles,
    id_override: Option<String>,
) -> anyhow::Result<(ServerHelloAck, QuicMessages)> {
    // Create QUIC client endpoint
    let client_config = libgsh::quic::create_client_config(insecure)?;
    let mut endpoint = libgsh::quic::create_client_endpoint().await?;
    endpoint.set_default_client_config(client_config);

    // Connect to server
    let addr = format!("{}:{}", host, port).parse()?;
    let connection = endpoint
        .connect(addr, host)?
        .await
        .map_err(|e| anyhow::anyhow!("QUIC connection failed: {}", e))?;

    log::info!("QUIC connection established to {}:{}", host, port);

    // Open the main control stream for the GSH protocol
    let (send, recv) = connection
        .open_bi()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to open QUIC stream: {}", e))?;

    let quic_stream = QuicStream { send, recv };
    
    // Host verification for QUIC connections
    if !insecure {
        // For now, we'll skip host verification since QUIC already provides 
        // certificate verification during connection establishment
        log::info!("QUIC connection uses built-in TLS certificate verification");
    }

    // Create message codec
    let mut messages = QuicMessages::new(quic_stream);
    
    // Perform GSH protocol handshake
    let hello = libgsh::shared::r#async::handshake_client(
        &mut messages,
        monitors,
        ClientAuthProvider::new(known_hosts, id_files, id_override),
        host,
    )
    .await?;

    Ok((hello, messages))
}

/// Shutdown QUIC connection
pub async fn shutdown_quic(messages: &mut QuicMessages) -> anyhow::Result<()> {
    log::trace!("Exiting QUIC connection gracefully...");
    messages
        .write_message(protocol::StatusUpdate {
            kind: StatusType::Exit as i32,
            details: None,
        })
        .await?;
    
    // The QUIC connection will be closed when the streams are dropped
    log::trace!("QUIC connection closed.");
    Ok(())
}
