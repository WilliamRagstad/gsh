use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    sync::{atomic::AtomicBool, mpsc, Arc},
};

use rustls::{ServerConfig, ServerConnection, StreamOwned};
use shared::{
    prost::Message,
    protocol::{client_hello, status_update::StatusType, StatusUpdate, UserInput},
    ClientEvent, MessageCodec,
};

use super::service::SimpleService;

const DEFAULT_PORT: u16 = 1122;
type Messages = MessageCodec<StreamOwned<ServerConnection, TcpStream>>;

/// A simple server that handles client connections and manages the application service implementing the `SimpleService` trait.
/// The server listens for incoming connections and spawns a new thread for each new client.
///
/// # Example: Self-Signed
/// ```
/// let (key, private_key) = cert::self_signed(&["localhost"])?;
/// let config = ServerConfig::builder()
///     .with_no_client_auth()
///     .with_single_cert(vec![key.cert.der().clone()], private_key)?;
/// let server = SimpleServer::new(config);
/// server.serve()?
/// `````
#[derive(Debug, Clone)]
pub struct SimpleServer<ServiceT: SimpleService> {
    _service: std::marker::PhantomData<ServiceT>,
    config: ServerConfig,
}

impl<ServiceT: SimpleService> SimpleServer<ServiceT> {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            _service: std::marker::PhantomData,
            config,
        }
    }

    pub fn serve(self) -> Result<(), Box<dyn std::error::Error>> {
        self.serve_port(DEFAULT_PORT)
    }

    pub fn serve_port(self, port: u16) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(format!("[::]:{}", port))?;
        println!(
            "Graphical Shell server is listening on {}",
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
                ServiceT::initial_window_settings(),
            )?;
            let os: client_hello::Os = client.os.try_into().unwrap_or(client_hello::Os::Unknown);
            println!(
                "+ Client connected from {} on {:?} {}",
                addr, os, client.os_version
            );
            std::thread::spawn(move || {
                if let Err(e) = Self::handle_client(messages) {
                    log::error!("Error handling client {}: {}", addr, e);
                }
            });
        }
        Ok(())
    }

    fn handle_client(mut messages: Messages) -> Result<(), Box<dyn std::error::Error>> {
        // Set the socket to non-blocking mode
        // All calls to `read_message` will return immediately, even if no data is available
        messages.get_stream().sock.set_nonblocking(true)?;

        let (send_event, recv_event) = mpsc::channel::<ClientEvent>();
        let (send_frame, recv_frame) = mpsc::channel::<shared::protocol::FrameData>();
        let service_running = Arc::new(AtomicBool::new(true));
        let service_running2 = Arc::clone(&service_running);
        let service_thread = std::thread::spawn(move || {
            let service = ServiceT::new(send_frame, recv_event);
            if let Err(e) = service.main() {
                log::error!("Service thread error: {}", e);
            }
            service_running2.store(false, std::sync::atomic::Ordering::SeqCst);
        });

        while service_running.load(std::sync::atomic::Ordering::SeqCst) {
            // Read messages from the client connection
            // This is a non-blocking call, so it will return immediately even if no data is available
            match messages.read_message() {
                Ok(buf) => {
                    if let Ok(status_update) = StatusUpdate::decode(&buf[..]) {
                        log::trace!("StatusUpdate: {:?}", status_update);
                        let status = status_update.kind;
                        if status == StatusType::Exit as i32 {
                            log::trace!("Received graceful exit status, closing connection...");
                            messages.get_stream().conn.send_close_notify();
                            messages.get_stream().flush()?;
                            messages
                                .get_stream()
                                .sock
                                .shutdown(std::net::Shutdown::Both)?;
                            drop(messages);
                            break;
                        }
                        send_event.send(ClientEvent::StatusUpdate(status_update))?;
                    } else if let Ok(user_input) = UserInput::decode(&buf[..]) {
                        log::trace!("UserInput: {:?}", user_input);
                        send_event.send(ClientEvent::UserInput(user_input))?;
                    } else {
                        log::trace!("Received data: {:?}", &buf[..]);
                        log::trace!("Unknown message type, ignoring...");
                    }
                }
                Err(err) => match err.kind() {
                    std::io::ErrorKind::UnexpectedEof => {
                        log::trace!("Client force disconnected, closing connection...");
                        break;
                    }
                    std::io::ErrorKind::WouldBlock => (), // No data available yet, do nothing
                    _ => {
                        log::error!("Error reading message: {}", err);
                        break;
                    }
                },
            };
            // Read messages from the service thread
            // This is a non-blocking call, so it will return immediately even if no data is available
            match recv_frame.try_recv() {
                Ok(frame) => messages.write_message(frame)?,
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => (), // do nothing, just continue
                    mpsc::TryRecvError::Disconnected => {
                        service_running.store(false, std::sync::atomic::Ordering::SeqCst);
                        break;
                    }
                },
            }
        }
        // Gracefully exit the client handling loop
        // Send a status update to the service thread to indicate that the client has disconnected
        if service_running.load(std::sync::atomic::Ordering::SeqCst) {
            log::trace!("Client disconnected, exiting...");
            send_event.send(ClientEvent::StatusUpdate(StatusUpdate {
                kind: StatusType::Exit as i32,
                message: "Client disconnected".to_string(),
                code: 0,
            }))?;
        } else {
            log::trace!("Service disconnected, exiting...");
        }
        println!("- Client disconnected");
        if let Err(e) = service_thread.join() {
            log::error!("Service thread error: {:?}", e);
        }
        Ok(())
    }
}
