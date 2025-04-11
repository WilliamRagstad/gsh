use std::{net::TcpStream, sync::Arc};

use clap::Parser;
use rustls::{
    client::danger::{ServerCertVerified, ServerCertVerifier},
    crypto::{ring as provider, CryptoProvider},
    time_provider, ClientConnection, StreamOwned,
};
use shared::{prost::Message, protocol, MessageCodec};

type Messages = MessageCodec<StreamOwned<ClientConnection, TcpStream>>;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// The host to connect to.
    #[clap(value_parser)]
    host: String,
    /// The port to connect to.
    #[clap(short, long, default_value_t = 1122)]
    port: u16,
    /// Disable TLS server certificate verification.
    #[clap(long)]
    insecure: bool,
}

fn main() {
    // Parse command line arguments
    let args = Args::parse();

    // Connect to the server
    let mut messages = match connect_tls(&args.host, args.port, args.insecure) {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            return;
        }
    };

    // Finnish the handshake
    if let Err(e) = handshake(&mut messages) {
        eprintln!("Handshake failed: {}", e);
        return;
    }

    let user_input1 = protocol::UserInput {
        kind: protocol::user_input::InputType::KeyPress as i32,
        key_code: 65,
        delta: 0,
        x: 42,
        y: 1337,
    };

    println!("UserInput: {:?}", user_input1);
    println!("Encoded UserInput: {:?}", user_input1.encode_to_vec());
    messages
        .write_message(&user_input1.encode_to_vec())
        .unwrap();

    // Close the connection
    println!("Exiting gracefully...");
    let status_close = protocol::StatusUpdate {
        status: protocol::status_update::StatusType::Close as i32,
        message: "Goodbye".to_string(),
        code: 0,
    };
    println!("StatusUpdate: {:?}", status_close);
    println!("Encoded StatusUpdate: {:?}", status_close.encode_to_vec());
    messages.get_stream().conn.send_close_notify();
    messages
        .write_message(&status_close.encode_to_vec())
        .unwrap();
    messages
        .get_stream()
        .sock
        .shutdown(std::net::Shutdown::Both)
        .unwrap();
    println!("Connection closed.");
}

fn connect_tls(
    host: &str,
    port: u16,
    insecure: bool,
) -> Result<Messages, Box<dyn std::error::Error>> {
    let root_store = if insecure {
        rustls::RootCertStore::empty()
    } else {
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
    };

    let mut config = rustls::ClientConfig::builder_with_details(
        CryptoProvider {
            cipher_suites: vec![provider::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256],
            kx_groups: vec![provider::kx_group::X25519],
            ..provider::default_provider()
        }
        .into(),
        Arc::new(time_provider::DefaultTimeProvider),
    )
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_root_certificates(root_store)
    .with_no_client_auth();

    if insecure {
        config
            .dangerous()
            .set_certificate_verifier(Arc::new(NoCertificateVerification {}));
    }

    // let mut config = rustls::ClientConfig::builder()
    //     .with_root_certificates(root_store)
    //     .with_no_client_auth();

    println!("Connecting to {}:{}...", host, port);
    let server_name = host.to_string().try_into()?;
    let mut conn = rustls::ClientConnection::new(Arc::new(config), server_name)?;
    let mut sock = TcpStream::connect(format!("{}:{}", host, port))?;
    conn.complete_io(&mut sock)?; // Complete the handshake with the stream
    let tls_stream = rustls::StreamOwned::new(conn, sock);

    // Check if the handshake was successful
    if tls_stream.conn.is_handshaking() {
        return Err("Handshake failed".into());
    }

    Ok(Messages::new(tls_stream))
}

fn handshake(messages: &mut Messages) -> Result<(), Box<dyn std::error::Error>> {
    let client_hello = shared::protocol::ClientHello { version: 1 };
    let mut buf: Vec<u8> = Vec::new();
    client_hello.encode(&mut buf)?;
    println!("ClientHello: {:?}", client_hello);
    println!("Encoded ClientHello: {:?}", buf);
    messages.write_message(&buf)?;

    let response = messages.read_message()?;
    println!("Received response: {:?}", &response[..]);
    let server_hello = shared::protocol::ServerHelloAck::decode(&response[..])?;
    println!("ServerHello: {:?}", server_hello);

    if server_hello.version != 1 {
        return Err("Invalid server version".into());
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct NoCertificateVerification {}

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
        ]
    }

    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Always return a valid certificate verification result
        Ok(ServerCertVerified::assertion())
    }
}
