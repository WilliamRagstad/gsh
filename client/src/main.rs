use std::process::exit;

use clap::Parser;
use client::Client;
use shared::protocol::{
    window_settings::{self, WindowMode},
    WindowSettings,
};

mod client;
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
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    let args = Args::parse();

    println!("Connecting to {}:{}...", args.host, args.port);
    let (windows, format, messages) = network::connect_tls(&args.host, args.port, args.insecure)
        .unwrap_or_else(|e| {
            log::error!("Failed to connect: {}", e);
            exit(1);
        });
    println!("Successfully connected to server!");

    let mut client = Client::new(format, messages).unwrap_or_else(|e| {
        log::error!("Failed to create client: {}", e);
        exit(1);
    });

    if windows.is_empty() {
        log::warn!("No initial window settings provided, creating a default window.");
        client
            .create_window(&default_window(args.host))
            .unwrap_or_else(|e| {
                log::error!("Failed to create default window: {}", e);
                exit(1);
            });
    } else {
        log::info!("Creating {} windows...", windows.len());
        for ws in windows {
            client.create_window(&ws).unwrap_or_else(|e| {
                log::error!("Failed to create window: {}", e);
                exit(1);
            });
        }
    }
    if let Err(e) = client.main() {
        log::error!("Client error: {}", e);
        exit(1);
    }

    let _ = network::shutdown_tls(client.messages());
}

fn default_window(host: String) -> WindowSettings {
    WindowSettings {
        window_id: 0,
        title: format!("GSH Client: {}", host),
        initial_mode: WindowMode::Windowed as i32,
        width: 800,
        height: 600,
        always_on_top: false,
        allow_resize: true,
        resize_frame: false,
        anchor: window_settings::FrameAnchor::TopLeft as i32,
    }
}
