use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    sync::Arc,
};

use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::{
    pki_types::{pem::PemObject, PrivateKeyDer},
    server::ServerConfig,
    ServerConnection, StreamOwned,
};
use shared::{prost::Message, MessageCodec};
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
    loop {
        let buf = match messages.read_message() {
            Ok(buf) => buf,
            Err(err) => match err.kind() {
                std::io::ErrorKind::UnexpectedEof => {
                    println!("Client force disconnected, closing connection...");
                    break;
                }
                _ => {
                    eprintln!("Error reading message: {}", err);
                    break;
                }
            },
        };
        println!("Received data: {:?}", &buf[..]);
        if let Ok(status_update) = shared::protocol::StatusUpdate::decode(&buf[..]) {
            println!("StatusUpdate: {:?}", status_update);
            if status_update.status == shared::protocol::status_update::StatusType::Close as i32 {
                println!("Received graceful close status, closing connection...");
                messages.get_stream().conn.send_close_notify();
                messages.get_stream().flush()?; // Ensure the close_notify is sent
                messages
                    .get_stream()
                    .sock
                    .shutdown(std::net::Shutdown::Both)?;
                drop(messages); // Drop the messages object to close the connection
                break;
            }
        } else if let Ok(user_input) = shared::protocol::UserInput::decode(&buf[..]) {
            println!("UserInput: {:?}", user_input);
            // Process user input here
        } else {
            println!("Unknown message type, ignoring...");
        }
    }
    Ok(())
}
