use anyhow::Result;
use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::{
    pki_types::{pem::PemObject, PrivateKeyDer},
    server::ServerConfig,
    ServerConnection, StreamOwned,
};
use shared::{
    prost::Message,
    protocol::{self, StatusUpdate},
    MessageCodec,
};
use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    sync::{mpsc, Arc},
};

mod service;
type Messages = MessageCodec<StreamOwned<ServerConnection, TcpStream>>;

const PORT: u16 = 1122;

fn main() {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    if let Err(e) = server() {
        log::error!("Failed to start server: {}", e);
    }
}

fn server() -> Result<()> {
    // Generate a self-signed certificate for the server
    let subject_alt_names = vec!["hello.world.example".to_string(), "localhost".to_string()];
    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(subject_alt_names).unwrap();
    let private_key = PrivateKeyDer::from_pem_slice(key_pair.serialize_pem().as_bytes())
        .expect("Failed to parse private key PEM");
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], private_key)?;

    let listener = TcpListener::bind(format!("[::]:{}", PORT))?;
    println!("GSH server is listening on {}", listener.local_addr()?);
    while let Ok((mut stream, addr)) = listener.accept() {
        let mut conn = ServerConnection::new(Arc::new(config.clone()))?;
        conn.complete_io(&mut stream)?; // Complete the handshake with the stream
        let tls_stream = StreamOwned::new(conn, stream);
        let mut messages = Messages::new(tls_stream);
        let initial_window_settings = service::Service::initial_window_settings();
        let client = shared::handshake_server(
            &mut messages,
            &[shared::PROTOCOL_VERSION],
            Some(initial_window_settings),
        )?;
        let os: protocol::client_hello::Os = client
            .os
            .try_into()
            .unwrap_or(protocol::client_hello::Os::Unknown);
        println!(
            "+ Client connected from {} on {:?} version {}",
            addr, os, client.os_version
        );
        std::thread::spawn(move || {
            if let Err(e) = handle_client(messages) {
                log::error!("Error handling client {}: {}", addr, e);
            }
        });
    }
    Ok(())
}

fn handle_client(mut messages: Messages) -> Result<()> {
    // Set the socket to non-blocking mode
    // All calls to `read_message` will return immediately, even if no data is available
    messages.get_stream().sock.set_nonblocking(true)?;

    let (event_send, event_recv) = mpsc::channel::<shared::ClientEvent>();
    let (frame_send, frame_recv) = mpsc::channel::<shared::protocol::FrameData>();
    let service_thread = std::thread::spawn(move || {
        let service = service::Service::new(frame_send, event_recv);
        if let Err(e) = service.main() {
            log::error!("Service thread error: {}", e);
        }
    });

    loop {
        // Read messages from the client
        match messages.read_message() {
            Ok(buf) => {
                if let Ok(status_update) = shared::protocol::StatusUpdate::decode(&buf[..]) {
                    log::trace!("StatusUpdate: {:?}", status_update);
                    let status = status_update.kind;
                    if status == shared::protocol::status_update::StatusType::Exit as i32 {
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
                    event_send.send(shared::ClientEvent::StatusUpdate(status_update))?;
                } else if let Ok(user_input) = shared::protocol::UserInput::decode(&buf[..]) {
                    log::trace!("UserInput: {:?}", user_input);
                    event_send.send(shared::ClientEvent::UserInput(user_input))?;
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
        // Read messages from the service
        match frame_recv.try_recv() {
            Ok(frame) => messages.write_message(frame)?,
            Err(e) => match e {
                mpsc::TryRecvError::Empty => (), // do nothing, just continue
                mpsc::TryRecvError::Disconnected => {
                    log::trace!("Service disconnected, exiting...");
                    break;
                }
            },
        }
    }
    event_send.send(shared::ClientEvent::StatusUpdate(StatusUpdate {
        kind: shared::protocol::status_update::StatusType::Exit as i32,
        message: "Client disconnected".to_string(),
        code: 0,
    }))?;
    log::trace!("Exiting client handler loop...");
    println!("- Client disconnected");
    // Wait for the service thread to finish
    if let Err(e) = service_thread.join() {
        log::error!("Service thread error: {:?}", e);
    }
    Ok(())
}
