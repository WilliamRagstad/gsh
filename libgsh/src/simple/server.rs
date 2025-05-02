use super::service::SimpleService;
use crate::shared::protocol::{self, client_hello};
use crate::{simple::Messages, Result};
use std::{net::TcpListener, sync::Arc};
use tokio_rustls::rustls::{ServerConfig, ServerConnection, StreamOwned};

const DEFAULT_PORT: u16 = 1122;

/// A simple server that handles client connections and manages the application service implementing the `SimpleService` trait.
/// The server listens for incoming connections and spawns a new thread for each new client.
///
/// # Example: Self-Signed
/// ```ignore
/// let (key, private_key) = crate::cert::self_signed(&["localhost"])?;
/// let config = ServerConfig::builder()
///     .with_no_client_auth()
///     .with_single_cert(vec![key.cert.der().clone()], private_key)?;
/// let server = SimpleServer::new(config);
/// server.serve()?
/// ```
#[derive(Debug, Clone)]
pub struct SimpleServer<ServiceT: SimpleService> {
    _service: std::marker::PhantomData<ServiceT>,
    config: ServerConfig,
}

impl<ServiceT: SimpleService> SimpleServer<ServiceT> {
    /// Creates a new `SimpleServer` instance with the provided server configuration.\
    /// The `ServerConfig` is used to configure the TLS settings for the server.
    pub fn new(config: ServerConfig) -> Self {
        Self {
            _service: std::marker::PhantomData,
            config,
        }
    }

    /// Starts the server and listens for incoming connections on the default port (1122).\
    /// This method blocks until the server is stopped or an error occurs.
    pub fn serve(self) -> Result<()> {
        self.serve_port(DEFAULT_PORT)
    }

    /// Starts the server and listens for incoming connections on the specified port.\
    /// This method blocks until the server is stopped or an error occurs.
    pub fn serve_port(self, port: u16) -> Result<()> {
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
        loop {
            let (mut stream, addr) = listener.accept()?;
            let mut conn = ServerConnection::new(Arc::new(self.config.clone()))?;
            std::thread::spawn(move || {
                conn.complete_io(&mut stream).unwrap();
                let tls_stream = StreamOwned::new(conn, stream);
                let messages = Messages::new(tls_stream);
                if let Err(e) = Self::handle_client(messages, addr) {
                    log::error!("Service error {}: {}", addr, e);
                }
                println!("- Client disconnected from {}", addr);
            });
        }
    }

    /// Handles a client connection.\
    /// This function performs the TLS handshake and starts the service's main event loop.\
    fn handle_client(mut messages: Messages, addr: std::net::SocketAddr) -> Result<()> {
        let client = crate::shared::sync::handshake_server(
            &mut messages,
            &[crate::shared::PROTOCOL_VERSION],
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

        // Verify ClientAuth message if auth_method is set
        if let Some(auth_method) = ServiceT::server_hello().auth_method {
            let client_auth = protocol::ClientAuth::decode(messages.read_message()?)?;
            match auth_method {
                protocol::server_hello_ack::AuthMethod::PASSWORD => {
                    let expected_password = "expected_password".to_string(); // Replace with actual expected password
                    if client_auth.password != Some(expected_password) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "Invalid password",
                        ));
                    }
                }
                protocol::server_hello_ack::AuthMethod::SIGNATURE => {
                    let expected_signature = vec![0u8; 64]; // Replace with actual expected signature
                    if client_auth.signature != Some(expected_signature) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "Invalid signature",
                        ));
                    }
                }
                _ => {}
            }
        }

        let service = ServiceT::new();
        service.main(messages)?;
        Ok(())
    }
}
