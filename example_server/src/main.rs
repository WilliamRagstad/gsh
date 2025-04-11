use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    sync::{mpsc, Arc},
};

use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::{
    pki_types::{pem::PemObject, PrivateKeyDer},
    server::ServerConfig,
    ServerConnection, StreamOwned,
};
use shared::{prost::Message, MessageCodec};

mod service;
type Messages = MessageCodec<StreamOwned<ServerConnection, TcpStream>>;

const PORT: u16 = 1122;

fn main() {
    if let Err(e) = server() {
        eprintln!("Failed to start server: {}", e);
    }
}

fn server() -> Result<(), Box<dyn std::error::Error>> {
    // Generate a self-signed certificate for the server
    let subject_alt_names = vec!["hello.world.example".to_string(), "localhost".to_string()];
    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(subject_alt_names).unwrap();
    let private_key = PrivateKeyDer::from_pem_slice(key_pair.serialize_pem().as_bytes())
        .expect("Failed to parse private key PEM");
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], private_key)?;

    let listener = TcpListener::bind(format!("[::]:{}", PORT))?;
    println!("Listening on {}", listener.local_addr()?);
    while let Ok((mut stream, addr)) = listener.accept() {
        let mut conn = ServerConnection::new(Arc::new(config.clone()))?;
        conn.complete_io(&mut stream)?; // Complete the handshake with the stream
        let tls_stream = StreamOwned::new(conn, stream);
        let messages = Messages::new(tls_stream);
        println!("\nHandling new client connection from {}", addr);
        if let Err(e) = handle_client(messages) {
            eprintln!("Error handling client {}: {}", addr, e);
        }
    }
    Ok(())
}

fn handle_client(mut messages: Messages) -> Result<(), Box<dyn std::error::Error>> {
    shared::handshake_server(&mut messages)?;
    // Set the socket to non-blocking mode
    // All calls to `read_message` will return immediately, even if no data is available
    messages.get_stream().sock.set_nonblocking(true)?;
    let (event_send, event_recv) = mpsc::channel::<service::ClientEvent>();
    let (frame_send, frame_recv) = mpsc::channel::<shared::protocol::FrameData>();

    let service = service::Service::new(frame_send, event_recv);
    let service_thread = std::thread::spawn(move || {
        if let Err(e) = service.main() {
            eprintln!("Service thread error: {}", e);
        }
    });

    loop {
        // Read messages from the client
        match messages.read_message() {
            Ok(buf) => {
                println!("Received data: {:?}", &buf[..]);
                if let Ok(status_update) = shared::protocol::StatusUpdate::decode(&buf[..]) {
                    println!("StatusUpdate: {:?}", status_update);
                    if status_update.status
                        == shared::protocol::status_update::StatusType::Exit as i32
                    {
                        println!("Received graceful exit status, closing connection...");
                        messages.get_stream().conn.send_close_notify();
                        messages.get_stream().flush()?;
                        messages
                            .get_stream()
                            .sock
                            .shutdown(std::net::Shutdown::Both)?;
                        drop(messages);
                        break;
                    }
                    event_send.send(service::ClientEvent::StatusUpdate(status_update))?;
                } else if let Ok(user_input) = shared::protocol::UserInput::decode(&buf[..]) {
                    println!("UserInput: {:?}", user_input);
                    event_send.send(service::ClientEvent::UserInput(user_input))?;
                } else {
                    println!("Unknown message type, ignoring...");
                }
            }
            Err(err) => match err.kind() {
                std::io::ErrorKind::UnexpectedEof => {
                    println!("Client force disconnected, closing connection...");
                    break;
                }
                std::io::ErrorKind::WouldBlock => (), // No data available yet, do nothing
                _ => {
                    eprintln!("Error reading message: {}", err);
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
                    println!("Service disconnected, exiting...");
                    break;
                }
            },
        }
    }
    println!("Exiting event loop...");
    // Wait for the service thread to finish
    if let Err(e) = service_thread.join() {
        eprintln!("Service thread error: {:?}", e);
    }
    Ok(())
}
