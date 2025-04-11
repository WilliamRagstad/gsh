use clap::Parser;
use network::Messages;
use shared::{prost::Message, protocol};

mod network;

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
    let mut messages =
        network::connect_tls(&args.host, args.port, args.insecure).unwrap_or_else(|e| {
            eprintln!("Failed to connect: {}", e);
            std::process::exit(1);
        });

    // Finnish the handshake
    if let Err(e) = shared::handshake_client(&mut messages) {
        eprintln!("Handshake failed: {}", e);
        return;
    }

    // let user_input1 = protocol::UserInput {
    //     kind: protocol::user_input::InputType::KeyPress as i32,
    //     key_code: 65,
    //     delta: 0,
    //     x: 42,
    //     y: 1337,
    // };

    // println!("UserInput: {:?}", user_input1);
    // messages.write_message(user_input1).unwrap();

    if let Err(e) = event_loop(&mut messages) {
        eprintln!("Event loop failed: {}", e);
        return;
    }

    // Shutdown the connection
    if let Err(e) = network::shutdown_tls(messages) {
        eprintln!("Failed to shut down: {}", e);
    }
}

fn event_loop(messages: &mut Messages) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let buf = match messages.read_message() {
            Ok(buf) => buf,
            Err(err) => match err.kind() {
                std::io::ErrorKind::UnexpectedEof => {
                    println!("Server force disconnected, closing connection...");
                    break;
                }
                _ => {
                    eprintln!("Error reading message: {}", err);
                    break;
                }
            },
        };
        println!("Received data: {:?}", &buf[..]);
        if let Ok(status_update) = protocol::StatusUpdate::decode(&buf[..]) {
            println!("StatusUpdate: {:?}", status_update);
            if status_update.status == protocol::status_update::StatusType::Exit as i32 {
                println!("Received graceful exit status, closing connection...");
                break;
            }
        } else {
            println!("Failed to decode message");
        }
    }
    println!("Exiting event loop...");
    Ok(())
}
