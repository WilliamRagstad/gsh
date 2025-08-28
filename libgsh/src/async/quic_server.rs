use super::service::AsyncService;
use crate::r#async::Messages;
use crate::shared::protocol::client_hello;
use crate::Result;
use std::net::SocketAddr;
use quinn::{Endpoint, RecvStream, SendStream, ServerConfig};

const DEFAULT_PORT: u16 = 1122;

/// QUIC stream wrapper that implements AsyncRead + AsyncWrite + Send + Unpin
pub struct QuicStreamWrapper {
    send: SendStream,
    recv: RecvStream,
}

impl QuicStreamWrapper {
    pub fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv }
    }
}

impl tokio::io::AsyncRead for QuicStreamWrapper {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::result::Result<(), std::io::Error>> {
        use std::pin::Pin;
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for QuicStreamWrapper {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::result::Result<usize, std::io::Error>> {
        use std::pin::Pin;
        match Pin::new(&mut self.send).poll_write(cx, buf) {
            std::task::Poll::Ready(Ok(n)) => std::task::Poll::Ready(Ok(n)),
            std::task::Poll::Ready(Err(e)) => {
                std::task::Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), std::io::Error>> {
        use std::pin::Pin;
        match Pin::new(&mut self.send).poll_flush(cx) {
            std::task::Poll::Ready(Ok(())) => std::task::Poll::Ready(Ok(())),
            std::task::Poll::Ready(Err(e)) => {
                std::task::Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), std::io::Error>> {
        use std::pin::Pin;
        match Pin::new(&mut self.send).poll_shutdown(cx) {
            std::task::Poll::Ready(Ok(())) => std::task::Poll::Ready(Ok(())),
            std::task::Poll::Ready(Err(e)) => {
                std::task::Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// A mock TLS-like interface for QUIC streams to work with existing service code
pub struct QuicTlsAdapter {
    stream: QuicStreamWrapper,
}

impl QuicTlsAdapter {
    pub fn new(stream: QuicStreamWrapper) -> Self {
        Self { stream }
    }
    
    pub fn get_mut(&mut self) -> (&mut QuicStreamWrapper, &mut QuicTlsMock) {
        (&mut self.stream, &mut QuicTlsMock)
    }
    
    pub fn get_ref(&self) -> (&QuicStreamWrapper, &QuicTlsMock) {
        (&self.stream, &QuicTlsMock)
    }
}

/// Mock TLS layer for QUIC (QUIC already provides TLS)
pub struct QuicTlsMock;

impl QuicTlsMock {
    pub fn send_close_notify(&mut self) {
        // QUIC handles connection closure automatically
        // No explicit close notify needed
    }
}

impl tokio::io::AsyncRead for QuicTlsAdapter {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::result::Result<(), std::io::Error>> {
        use std::pin::Pin;
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for QuicTlsAdapter {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::result::Result<usize, std::io::Error>> {
        use std::pin::Pin;
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), std::io::Error>> {
        use std::pin::Pin;
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), std::io::Error>> {
        use std::pin::Pin;
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// Asynchronous message codec for the QUIC stream
pub type QuicMessages = crate::shared::r#async::AsyncMessageCodec<QuicTlsAdapter>;

/// An async QUIC server that handles client connections and manages the application service implementing the `AsyncService` trait.
/// The server listens for incoming QUIC connections and spawns a new task for each client connection.
#[derive(Debug, Clone)]
pub struct AsyncQuicServer<ServiceT: AsyncService> {
    service: ServiceT,
    config: ServerConfig,
}

impl<ServiceT: AsyncService> AsyncQuicServer<ServiceT>
where
    ServiceT: Send + Sync + 'static,
{
    /// Creates a new `AsyncQuicServer` instance with the provided QUIC server configuration.
    pub fn new(service: ServiceT, config: ServerConfig) -> Self {
        Self { service, config }
    }

    /// Starts the server and listens for incoming connections on the default port (1122).
    pub async fn serve(self) -> Result<()> {
        self.serve_port(DEFAULT_PORT).await
    }

    /// Starts the server and listens for incoming connections on the specified port.
    pub async fn serve_port(self, port: u16) -> Result<()> {
        let addr: SocketAddr = format!("[::]:{}",port).parse().unwrap();
        let endpoint = Endpoint::server(self.config.clone(), addr)?;
        
        let service_fullname = std::any::type_name::<ServiceT>();
        let service_name = service_fullname
            .split("::")
            .last()
            .unwrap_or(service_fullname);
        
        println!(
            "Graphical Shell QUIC server running {} is listening on {}",
            service_name, addr
        );

        loop {
            let incoming_conn = endpoint.accept().await;
            if let Some(conn) = incoming_conn {
                let service = self.service.clone();
                tokio::spawn(async move {
                    match conn.await {
                        Ok(connection) => {
                            let addr = connection.remote_address();
                            log::info!("QUIC connection established from {}", addr);
                            
                            // Accept the bidirectional stream
                            match connection.accept_bi().await {
                                Ok((send, recv)) => {
                                    let stream = QuicStreamWrapper::new(send, recv);
                                    let adapter = QuicTlsAdapter::new(stream);
                                    let messages = QuicMessages::new(adapter);
                                    if let Err(e) = Self::handle_client(service, messages, addr).await {
                                        log::error!("QUIC Service error {}: {}", addr, e);
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to accept QUIC stream from {}: {}", addr, e);
                                }
                            }
                            println!("- QUIC Client disconnected from {}", addr);
                        }
                        Err(e) => {
                            log::error!("QUIC connection failed: {}", e);
                        }
                    }
                });
            }
        }
    }

    /// Handles a client connection over QUIC.
    /// This function performs the GSH protocol handshake and starts the service's main event loop.
    async fn handle_client(
        service: ServiceT,
        mut messages: QuicMessages,
        addr: SocketAddr,
    ) -> Result<()> {
        let client = crate::shared::r#async::handshake_server(
            &mut messages,
            &[crate::shared::PROTOCOL_VERSION],
            service.server_hello(),
            service.auth_verifier(),
        )
        .await?;
        let os: client_hello::Os = client.os.try_into().unwrap_or(client_hello::Os::Unknown);
        let monitors = client.monitors.len();
        log::info!(
            "+ QUIC Client connected running {:?} {} with {} monitor(s) on {}",
            os,
            client.os_version,
            monitors,
            addr.port()
        );

        // Convert QuicMessages to Messages for the service
        // This is a bit of a hack, but it allows us to use the existing service interface
        // TODO: Make the service trait generic over the stream type
        let tls_messages = unsafe { 
            std::mem::transmute::<QuicMessages, Messages>(messages) 
        };
        
        service.main(tls_messages).await?;
        Ok(())
    }
}