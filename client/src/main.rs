use anyhow::Result;
use clap::Parser;
use network::Messages;
use shared::{
    prost::Message,
    protocol::{self, StatusUpdate, WindowSettings},
};
use std::sync::mpsc;

mod network;
mod window;

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
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    if let Err(e) = client(Args::parse()) {
        log::error!("Failed to start client: {}", e);
    }
}

fn client(args: Args) -> Result<()> {
    // let (event_send, event_recv) = mpsc::channel::<shared::ClientEvent>();
    // let (frame_send, frame_recv) = mpsc::channel::<shared::protocol::FrameData>();
    // let client_window = window::ClientWindow::new(event_send, frame_recv);
    // client_window.main()?;

    // let user_input1 = protocol::UserInput {
    //     kind: protocol::user_input::InputType::KeyPress as i32,
    //     key_code: 65,
    //     delta: 0,
    //     x: 42,
    //     y: 1337,
    // };

    // log::info!("UserInput: {:?}", user_input1);
    // messages.write_message(user_input1).unwrap();

    // Connect to the server
    println!("Connecting to {}:{}...", args.host, args.port);
    let (initial_window_settings, mut messages) =
        network::connect_tls(&args.host, args.port, args.insecure)?;
    println!("Successfully connected to server!");
    if let Err(e) = event_loop(&mut messages, initial_window_settings, args.host) {
        log::error!("Error in event loop: {}", e);
    }
    let _ = network::shutdown_tls(messages);
    Ok(())
}

fn event_loop(
    messages: &mut Messages,
    initial_window_settings: Option<WindowSettings>,
    host: String,
) -> Result<()> {
    // Set the socket to non-blocking mode
    // All calls to `read_message` will return immediately, even if no data is available
    messages.get_stream().sock.set_nonblocking(true)?;

    let (event_send, event_recv) = mpsc::channel::<shared::ClientEvent>();
    let (frame_send, frame_recv) = mpsc::channel::<shared::protocol::FrameData>();
    let wnd_thread = std::thread::spawn(move || {
        let wnd = window::ClientWindow::new(event_send, frame_recv, initial_window_settings, host);
        if let Err(e) = wnd.main() {
            log::error!("Window thread error: {}", e);
        }
    });

    loop {
        // Read messages from the server
        match messages.read_message() {
            Ok(buf) => {
                if let Ok(status_update) = protocol::StatusUpdate::decode(&buf[..]) {
                    if !handle_status_update(status_update) {
                        break;
                    }
                }
                if let Ok(frame) = protocol::FrameData::decode(&buf[..]) {
                    frame_send.send(frame)?;
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

        // Read messages from the client window
        match event_recv.try_recv() {
            Ok(msg) => match msg {
                shared::ClientEvent::StatusUpdate(status_update) => {
                    log::trace!("StatusUpdate: {:?}", status_update);
                    messages.write_message(status_update)?
                }
                shared::ClientEvent::UserInput(user_input) => {
                    log::trace!("UserInput: {:?}", user_input);
                    messages.write_message(user_input)?
                }
            },
            Err(e) => match e {
                mpsc::TryRecvError::Empty => (), // do nothing, just continue
                mpsc::TryRecvError::Disconnected => {
                    log::trace!("Client window disconnected, exiting...");
                    break;
                }
            },
        }
    }
    log::trace!("Exiting event loop...");
    if let Err(e) = wnd_thread.join() {
        log::error!("Window thread error: {:?}", e);
    }
    Ok(())
}

fn handle_status_update(su: StatusUpdate) -> bool {
    if su.kind == protocol::status_update::StatusType::Exit as i32 {
        log::trace!("Received graceful exit status, closing connection...");
        return false;
    } else {
        log::trace!("StatusUpdate: {:?}", su);
    }
    // continue the loop
    true
}
