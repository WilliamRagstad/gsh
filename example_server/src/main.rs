use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
};

use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::{
    pki_types::{pem::PemObject, PrivateKeyDer},
    server::ServerConfig,
    ServerConnection, StreamOwned,
};
use shared::prost::Message;
const PORT: u16 = 1122;

fn main() {
    println!("Hosting example GSH server on port {}", PORT);
    if let Err(e) = server() {
        eprintln!("Failed to start server: {}", e);
    }
}

fn server() -> Result<(), Box<dyn std::error::Error>> {
    // Generate a self-signed certificate for the server
    // Generate a certificate that's valid for "localhost" and "hello.world.example"
    let subject_alt_names = vec!["hello.world.example".to_string(), "localhost".to_string()];

    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(subject_alt_names).unwrap();
    println!("PEM:\n{}", cert.pem());
    println!("Serialized:\n{}", key_pair.serialize_pem());

    let private_key = PrivateKeyDer::from_pem_slice(key_pair.serialize_pem().as_bytes())
        .expect("Failed to parse private key PEM");

    // Create a TLS server configuration with the generated certificate and key
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], private_key)?;

    // Set the server's port and address
    let listener = TcpListener::bind(format!("[::]:{}", PORT))?;
    println!("Listening on {}", listener.local_addr()?);

    // Accept incoming connections and handle them
    while let Ok((mut stream, addr)) = listener.accept() {
        println!("Accepted connection from {}", addr);
        let mut conn = ServerConnection::new(Arc::new(config.clone()))?;
        conn.complete_io(&mut stream)?; // Complete the handshake with the stream
        println!("Handshake completed with {}", addr);
        let tls_stream = StreamOwned::new(conn, stream);

        // Handle the client connection in a separate thread or async task
        println!("\nHandling new client connection...");
        if let Err(e) = handle_client(tls_stream) {
            eprintln!("Error handling client {}: {}", addr, e);
        }
    }

    Ok(())
}

fn handle_client(
    mut tls_stream: StreamOwned<ServerConnection, TcpStream>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Handle the client connection here
    // For example, read data from the stream and process it
    let mut buf = vec![0; 1024]; // Adjust the buffer size as needed
    let bytes_read = tls_stream.read(&mut buf)?;
    buf.truncate(bytes_read); // Resize the buffer to the actual number of bytes read

    println!("Received data: {:?}", &buf[..]);
    let client_hello = shared::protocol::ClientHello::decode(&buf[..])?;
    println!("ClientHello: {:?}", client_hello);

    let server_hello = shared::protocol::ServerHelloAck { version: 1 };
    println!("ServerHello: {:?}", server_hello);
    println!("Encoded ServerHello: {:?}", server_hello.encode_to_vec());
    tls_stream.write_all(&server_hello.encode_to_vec())?;

    loop {
        let bytes_read = tls_stream.read(&mut buf)?;
        if bytes_read == 0 {
            println!("Client force disconnected, closing connection...");
            break;
        }
        buf.truncate(bytes_read); // Resize the buffer to the actual number of bytes read
        println!("Received data: {:?}", &buf[..]);
        if let Ok(status_update) = shared::protocol::StatusUpdate::decode(&buf[..]) {
            println!("StatusUpdate: {:?}", status_update);
            if status_update.status == shared::protocol::status_update::StatusType::Close as i32 {
                println!("Received graceful close status, closing connection...");
                tls_stream.conn.send_close_notify();
                tls_stream.flush()?; // Ensure the close_notify is sent
                let _ = tls_stream.sock.shutdown(std::net::Shutdown::Both);
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
