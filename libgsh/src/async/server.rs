use super::service::AsyncService;
use crate::r#async::Messages;
use crate::Result;
use shared::protocol::client_hello;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

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
pub struct AsyncServer<ServiceT: AsyncService> {
    _service: std::marker::PhantomData<ServiceT>,
    config: ServerConfig,
}

impl<ServiceT: AsyncService> AsyncServer<ServiceT>
where
    ServiceT: Send + Sync + 'static,
{
    /// Creates a new `AsyncServer` instance with the provided server configuration.\
    /// The `ServerConfig` is used to configure the TLS settings for the server.
    pub fn new(config: ServerConfig) -> Self {
        Self {
            _service: std::marker::PhantomData,
            config,
        }
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
            tokio::spawn(async move {
                let tls_stream = tls_acceptor.accept(stream).await.unwrap();
                let messages = Messages::new(tls_stream);
                if let Err(e) = Self::handle_client(messages, addr).await {
                    log::error!("Service error {}: {}", addr, e);
                }
                println!("- Client disconnected from {}", addr);
            });
        }
    }

    /// Handles a client connection.\
    /// This function performs the TLS handshake and starts the service's main event loop.\
    async fn handle_client(mut messages: Messages, addr: std::net::SocketAddr) -> Result<()> {
        let client = shared::r#async::handshake_server(
            &mut messages,
            &[shared::PROTOCOL_VERSION],
            ServiceT::server_hello(),
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
        let service = ServiceT::new();
        service.main(messages).await?;
        Ok(())
    }
}
