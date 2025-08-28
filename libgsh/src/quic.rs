//! QUIC networking module for GSH protocol
//!
//! This module provides QUIC-based client and server implementations that work alongside
//! the existing TCP+TLS implementations. QUIC provides built-in TLS 1.3 encryption and
//! supports multiple streams for better performance.
//!
//! QUIC Multi-Stream Architecture:
//! - Stream 0 (bidirectional): Control messages, handshake, status updates
//! - Stream 1+ (unidirectional): Frame data for better performance
//! - This allows frames and control messages to be sent independently

use std::sync::Arc;
use std::net::SocketAddr;
use anyhow::Result;
use quinn::{ClientConfig, Endpoint, ServerConfig, Connection};
use tokio_rustls::rustls;
use std::collections::HashMap;

/// QUIC connection manager that handles multiple streams
pub struct QuicConnection {
    connection: Connection,
    control_stream: Option<(quinn::SendStream, quinn::RecvStream)>,
    frame_streams: HashMap<u64, quinn::SendStream>,
    next_stream_id: u64,
}

impl QuicConnection {
    pub fn new(connection: Connection) -> Self {
        Self {
            connection,
            control_stream: None,
            frame_streams: HashMap::new(),
            next_stream_id: 1,
        }
    }
    
    /// Get or create the main control stream (bidirectional stream 0)
    pub async fn control_stream(&mut self) -> Result<&mut (quinn::SendStream, quinn::RecvStream)> {
        if self.control_stream.is_none() {
            let (send, recv) = self.connection.open_bi().await?;
            self.control_stream = Some((send, recv));
        }
        Ok(self.control_stream.as_mut().unwrap())
    }
    
    /// Create a new unidirectional stream for frame data
    pub async fn create_frame_stream(&mut self) -> Result<&quinn::SendStream> {
        let stream = self.connection.open_uni().await?;
        let stream_id = self.next_stream_id;
        self.next_stream_id += 1;
        self.frame_streams.insert(stream_id, stream);
        Ok(self.frame_streams.get(&stream_id).unwrap())
    }
    
    /// Accept incoming streams (for server side)
    pub async fn accept_bi(&self) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        self.connection.accept_bi().await.map_err(Into::into)
    }
    
    /// Accept incoming unidirectional streams (for server side)
    pub async fn accept_uni(&self) -> Result<quinn::RecvStream> {
        self.connection.accept_uni().await.map_err(Into::into)
    }
}

/// Enhanced QUIC client configuration with multi-stream support
pub fn create_client_config_with_streams(insecure: bool) -> Result<ClientConfig> {
    create_client_config(insecure)
}

/// Client configuration for QUIC connections
pub fn create_client_config(insecure: bool) -> Result<ClientConfig> {
    let root_store = if insecure {
        rustls::RootCertStore::empty()
    } else {
        // Use the same approach as the existing TLS code
        let mut roots = rustls::RootCertStore::empty();
        for cert in rustls_native_certs::load_native_certs().certs {
            let _ = roots.add(cert);
        }
        roots
    };

    let mut client_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    if insecure {
        // Skip certificate verification for insecure connections
        client_config.dangerous()
            .set_certificate_verifier(Arc::new(SkipServerVerification));
    }

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_config)?
    ));
    
    Ok(client_config)
}

/// Server configuration for QUIC connections
pub fn create_server_config(cert_chain: Vec<rustls::pki_types::CertificateDer<'static>>, 
                          private_key: rustls::pki_types::PrivateKeyDer<'static>) -> Result<ServerConfig> {
    let server_config = quinn::ServerConfig::with_single_cert(
        cert_chain, 
        private_key
    )?;
    
    Ok(server_config)
}

/// Create a QUIC endpoint for client connections
pub async fn create_client_endpoint() -> Result<Endpoint> {
    let endpoint = Endpoint::client("[::]:0".parse()?)?;
    Ok(endpoint)
}

/// Create a QUIC endpoint for server connections
pub async fn create_server_endpoint(addr: SocketAddr, server_config: ServerConfig) -> Result<Endpoint> {
    let endpoint = Endpoint::server(server_config, addr)?;
    Ok(endpoint)
}

/// Skip server certificate verification for insecure connections
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

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
            rustls::SignatureScheme::RSA_PKCS1_SHA1,
            rustls::SignatureScheme::ECDSA_SHA1_Legacy,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}