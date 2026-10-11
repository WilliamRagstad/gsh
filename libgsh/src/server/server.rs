use super::ServerStream;
use crate::{server::service::GshService, shared::protocol::client_hello, Result};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::{rustls::ServerConfig, TlsAcceptor};

const DEFAULT_PORT: u16 = 1122;

/// TLS server that clones a `GshService` into a task for each connection.
/// See the repository examples for TLS configuration and service implementations.
#[derive(Debug, Clone)]
pub struct GshServer<ServiceT: GshService> {
    service: ServiceT,
    config: ServerConfig,
}

impl<ServiceT: GshService> GshServer<ServiceT>
where
    ServiceT: Send + Sync + 'static,
{
    /// Creates a server with the service and TLS configuration.
    pub fn new(service: ServiceT, config: ServerConfig) -> Self {
        Self { service, config }
    }

    /// Listens on port 1122 until an error occurs or the task is cancelled.
    pub async fn serve(self) -> Result<()> {
        self.serve_port(DEFAULT_PORT).await
    }

    /// Listens on `port` until an error occurs or the task is cancelled.
    pub async fn serve_port(self, port: u16) -> Result<()> {
        let listener = TcpListener::bind(format!("[::]:{}", port)).await?;
        let tls_acceptor = TlsAcceptor::from(Arc::new(self.config.clone()));
        let service_fullname = std::any::type_name::<ServiceT>();
        let service_name = service_fullname
            .split("::")
            .last()
            .unwrap_or(service_fullname);
        println!(
            "Graphical Shell server running {} is listening on {}",
            service_name,
            listener.local_addr()?
        );
        loop {
            let (stream, addr) = listener.accept().await?;
            let tls_acceptor = tls_acceptor.clone();
            let service = self.service.clone();
            tokio::spawn(async move {
                let tls_stream = tls_acceptor.accept(stream).await.unwrap();
                let stream = ServerStream::new(tls_stream);
                if let Err(e) = Self::handle_client(service, stream, addr).await {
                    log::error!("Service error {}: {}", addr, e);
                }
                println!("- Client disconnected from {}", addr);
            });
        }
    }

    /// Negotiates the protocol and runs the per-client service.
    async fn handle_client(
        service: ServiceT,
        mut stream: ServerStream,
        addr: std::net::SocketAddr,
    ) -> Result<()> {
        let client = super::handshake::handshake(
            &mut stream,
            &[crate::shared::PROTOCOL_VERSION],
            service.server_hello(),
            service.auth_verifier(),
        )
        .await?;
        let os: client_hello::Os = client.os.try_into().unwrap_or(client_hello::Os::Unknown);
        let monitors = client.monitors.len();
        log::info!(
            "+ Client connected running {:?} {} with {} monitor(s) on {}",
            os,
            client.os_version,
            monitors,
            addr.port()
        );

        service.main(stream).await?;
        Ok(())
    }
}
