use std::{
    net::{TcpListener, TcpStream},
    sync::Arc,
};

use rustls::{ServerConfig, ServerConnection, StreamOwned};
use shared::{protocol::client_hello, MessageCodec};

use super::service::AsyncService;

const DEFAULT_PORT: u16 = 1122;
pub type Messages = MessageCodec<StreamOwned<ServerConnection, TcpStream>>;

/// An async server that handles client connections and manages the application service implementing the `AsyncService` trait.
/// The server listens for incoming connections and spawns a new tasks for each client connection.\
///
/// # Example: Self-Signed
/// ```
/// let (key, private_key) = cert::self_signed(&["localhost"])?;
/// let config = ServerConfig::builder()
///     .with_no_client_auth()
///     .with_single_cert(vec![key.cert.der().clone()], private_key)?;
/// let server = AsyncServer::new(config);
/// server.serve()?
/// `````
#[derive(Debug, Clone)]
pub struct AsyncServer<ServiceT: AsyncService> {
    _service: std::marker::PhantomData<ServiceT>,
    config: ServerConfig,
}

impl<ServiceT: AsyncService> AsyncServer<ServiceT> {
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
    pub fn serve(self) -> Result<(), Box<dyn std::error::Error>> {
        self.serve_port(DEFAULT_PORT)
    }

    /// Starts the server and listens for incoming connections on the specified port.\
    /// This method blocks until the server is stopped or an error occurs.
    pub fn serve_port(self, port: u16) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(format!("[::]:{}", port))?;
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
        while let Ok((mut stream, addr)) = listener.accept() {
            let mut conn = ServerConnection::new(Arc::new(self.config.clone()))?;
            conn.complete_io(&mut stream)?;
            let tls_stream = StreamOwned::new(conn, stream);
            let mut messages = Messages::new(tls_stream);
            let client = shared::handshake_server(
                &mut messages,
                &[shared::PROTOCOL_VERSION],
                ServiceT::server_hello(),
            )?;
            let os: client_hello::Os = client.os.try_into().unwrap_or(client_hello::Os::Unknown);
            let monitors = client.monitors.len();
            println!(
                "+ Client connected running {:?} {} with {} monitor(s) on {}",
                os,
                client.os_version,
                monitors,
                addr.port(),
            );
            std::thread::spawn(move || {
                if let Err(e) = ServiceT::new().main(messages) {
                    log::error!("Service error {}: {}", addr, e);
                }
                println!("- Client disconnected from {}", addr);
            });
        }
        Ok(())
    }
}
