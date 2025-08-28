//! Basic example demonstrating both TCP+TLS and QUIC connectivity options
//! 
//! This example shows how to create a simple service that can work with both
//! the traditional TCP+TLS and the new QUIC+TLS connections.

use libgsh::r#async::{server::AsyncServer, quic_server::AsyncQuicServer, service::AsyncService};
use libgsh::shared::protocol::{server_hello_ack, ServerHelloAck};
use libgsh::shared::auth::AuthVerifier;
use libgsh::cert::self_signed;
use libgsh::quic::{create_server_config};
use libgsh::{Result, r#async::Messages};
use async_trait::async_trait;
use tokio_rustls::rustls::ServerConfig;

/// A simple test service that demonstrates both TLS and QUIC connectivity
#[derive(Debug, Clone)]
pub struct SimpleTestService;

#[async_trait]
impl AsyncService for SimpleTestService {
    fn server_hello(&self) -> ServerHelloAck {
        ServerHelloAck {
            format: server_hello_ack::FrameFormat::Rgb.into(),
            compression: None,
            windows: Vec::new(),
            auth_method: None,
        }
    }
    
    fn auth_verifier(&self) -> Option<AuthVerifier> {
        None
    }
    
    async fn main(self, _messages: Messages) -> Result<()> {
        println!("Service is running with TLS connection!");
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    
    // Create self-signed certificate for both TLS and QUIC
    let (cert_key, private_key) = self_signed(&["localhost", "127.0.0.1"])
        .map_err(|e| anyhow::anyhow!("Failed to create certificate: {}", e))?;
    
    // TLS server configuration
    let tls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_key.cert.der().clone()], private_key.clone_key())?;
    
    // QUIC server configuration
    let quic_config = create_server_config(
        vec![cert_key.cert.der().clone()], 
        private_key.clone_key()
    )?;
    
    let service = SimpleTestService;
    
    // Create both servers
    let tls_server = AsyncServer::new(service.clone(), tls_config);
    let quic_server = AsyncQuicServer::new(service, quic_config);
    
    // Run both servers concurrently on different ports
    let tls_handle = tokio::spawn(async move {
        println!("Starting TLS server on port 1122...");
        tls_server.serve_port(1122).await
    });
    
    let quic_handle = tokio::spawn(async move {
        println!("Starting QUIC server on port 1123...");
        quic_server.serve_port(1123).await
    });
    
    println!("Both TLS and QUIC servers are starting...");
    println!("TLS server: localhost:1122");
    println!("QUIC server: localhost:1123");
    
    // Wait for either server to complete (they run forever unless there's an error)
    tokio::select! {
        result = tls_handle => {
            println!("TLS server finished: {:?}", result);
        }
        result = quic_handle => {
            println!("QUIC server finished: {:?}", result);
        }
    }
    
    Ok(())
}