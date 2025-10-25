use super::service::AsyncService;
use crate::shared::protocol::client_hello;
use crate::shared::r#async::AsyncMessageCodec;
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

/// Asynchronous message codec for the QUIC stream
pub type QuicMessages = AsyncMessageCodec<QuicStreamWrapper>;

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
                            
                            // Accept the bidirectional control stream
                            match connection.accept_bi().await {
                                Ok((send, recv)) => {
                                    log::info!("QUIC control stream established from {}", addr);
                                    let stream = QuicStreamWrapper::new(send, recv);
                                    let messages = QuicMessages::new(stream);
                                    
                                    // Spawn a task to handle additional frame streams
                                    let conn_clone = connection.clone();
                                    let addr_clone = addr;
                                    tokio::spawn(async move {
                                        Self::handle_frame_streams(conn_clone, addr_clone).await;
                                    });
                                    
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

        // For now, we'll need to create a QUIC-compatible version of the service main loop
        // This is a simplified implementation that bypasses the TLS-specific parts of AsyncServiceExt
        Self::quic_main_loop(service, messages).await?;
        Ok(())
    }

    /// A simplified main loop for QUIC services that doesn't depend on TLS-specific features
    async fn quic_main_loop(
        _service: ServiceT,
        _messages: QuicMessages,
    ) -> Result<()> {
        // Call the original main method - the service is responsible for handling the stream
        // Since AsyncService::main expects Messages (TLS), we need a way to adapt this
        // For now, let's create a simple event loop that works with QUIC
        log::trace!("Starting QUIC service main loop...");
        
        // TODO: Implement a proper QUIC-compatible service loop
        // For now, we'll just log that QUIC service is running
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            log::trace!("QUIC service running...");
            // This is a placeholder - real implementation would handle messages
            break;
        }
        
        log::trace!("QUIC Service main loop exited.");
        Ok(())
    }
    
    /// Handle additional QUIC streams for frame data
    async fn handle_frame_streams(connection: quinn::Connection, addr: SocketAddr) {
        loop {
            match connection.accept_uni().await {
                Ok(_recv_stream) => {
                    log::debug!("New QUIC frame stream from {}", addr);
                    // TODO: Handle frame data streams
                    // For now, just log that we received a frame stream
                    tokio::spawn(async move {
                        // Read frame data from this stream
                        log::trace!("Frame stream handler for {} started", addr);
                    });
                }
                Err(e) => {
                    log::debug!("No more QUIC frame streams from {}: {}", addr, e);
                    break;
                }
            }
        }
    }
}