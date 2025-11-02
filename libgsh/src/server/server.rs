use super::GshStream;
use crate::{server::service::GshService, shared::protocol::client_hello, Result};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::{rustls::ServerConfig, TlsAcceptor};

const DEFAULT_PORT: u16 = 1122;

/// An async server that handles client connections and manages the application service implementing the `AsyncService` trait.
/// The server listens for incoming connections and spawns a new tasks for each client connection.\
///
/// # Example: Self-Signed
/// ```ignore
/// let (key, private_key) = cert::self_signed(&["localhost"])?;
/// let config = ServerConfig::builder()
///     .with_no_client_auth()
///     .with_single_cert(vec![key.cert.der().clone()], private_key)?;
/// let server = AsyncServer::new(config);
/// server.serve()?
/// ```
#[derive(Debug, Clone)]
pub struct GshServer<ServiceT: GshService> {
    service: ServiceT,
    config: ServerConfig,
}

impl<ServiceT: GshService> GshServer<ServiceT>
where
    ServiceT: Send + Sync + 'static,
{
    /// Creates a new `GshServer` instance with the provided server configuration.\
    /// The `ServerConfig` is used to configure the TLS settings for the server.
    pub fn new(service: ServiceT, config: ServerConfig) -> Self {
        Self { service, config }
    }

    /// Starts the server and listens for incoming connections on the default port (1122).\
    /// This method blocks until the server is stopped or an error occurs.
    pub async fn serve(self) -> Result<()> {
        self.serve_port(DEFAULT_PORT).await
    }

    /// Starts the server and listens for incoming connections on the specified port.\
    /// This method blocks until the server is stopped or an error occurs.
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
                let stream = GshStream::new(tls_stream);
                if let Err(e) = Self::handle_client(service, stream, addr).await {
                    log::error!("Service error {}: {}", addr, e);
                }
                println!("- Client disconnected from {}", addr);
            });
        }
    }

    /// Handles a client connection.\
    /// This function performs the TLS handshake and starts the service's main event loop.\
    async fn handle_client(
        service: ServiceT,
        mut stream: GshStream,
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
