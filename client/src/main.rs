use std::process::exit;
use std::path::PathBuf;
use std::fs::File;
use std::io::Write;

use clap::{Parser, Subcommand};
use client::Client;
use libgsh::shared::protocol::{
    client_hello::MonitorInfo,
    server_hello_ack::{window_settings, window_settings::WindowMode, FrameFormat, WindowSettings},
};
use rsa::{RsaPrivateKey, RsaPublicKey, pkcs8::EncodePrivateKey, pkcs8::EncodePublicKey};
use rand::rngs::OsRng;

mod client;
mod config;
mod network;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// The host to connect to.
    #[clap(value_parser)]
    host: Option<String>,
    /// The port to connect to.
    #[clap(short, long, default_value_t = 1122)]
    port: u16,
    /// Disable TLS server certificate verification.
    #[clap(long)]
    insecure: bool,
    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a new named ID file
    CreateIdFile {
        /// The name of the ID file
        name: String,
    },
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error"))
        .format_line_number(true)
        .format_timestamp(None)
        .init();
    let args = Args::parse();

    if let Some(command) = args.command {
        match command {
            Command::CreateIdFile { name } => {
                let mut id_files = config::IdFiles::load();
                create_id_file(name, &mut id_files);
                return;
            }
        }
    }

    let mut known_hosts = config::KnownHosts::load();

    // Initialize SDL2
    let sdl = sdl2::init().unwrap_or_else(|e| {
        log::error!("Failed to initialize SDL2: {}", e);
        exit(1);
    });
    let video = sdl.video().unwrap_or_else(|e| {
        log::error!("Failed to initialize SDL2 video subsystem: {}", e);
        exit(1);
    });

    let host = args.host.unwrap_or_else(|| {
        log::error!("Host is required unless creating an ID file.");
        exit(1);
    });

    println!("Connecting to {}:{}...", host, args.port);
    let (hello, messages) = network::connect_tls(
        &host,
        args.port,
        args.insecure,
        monitor_info(&video),
        &mut known_hosts,
    )
    .await
    .unwrap_or_else(|e| {
        log::error!("Failed to connect: {}", e);
        exit(1);
    });
    let format: FrameFormat = hello.format.try_into().unwrap_or_else(|_| {
        log::error!("Failed to parse frame format: {}", hello.format);
        exit(1);
    });
    println!("Successfully connected to server!");

    let mut client = match Client::new(sdl, video, format, messages) {
        Ok(client) => client,
        Err(e) => {
            log::error!("Failed to create client: {}", e);
            exit(1);
        }
    };

    if hello.windows.is_empty() {
        log::warn!("No initial window settings provided, creating a default window.");
        client
            .create_window(&default_window(host))
            .unwrap_or_else(|e| {
                log::error!("Failed to create default window: {}", e);
                exit(1);
            });
    } else {
        log::info!("Creating {} windows...", hello.windows.len());
        for ws in hello.windows {
            client.create_window(&ws).unwrap_or_else(|e| {
                log::error!("Failed to create window: {}", e);
                exit(1);
            });
        }
    }
    if let Err(e) = client.main().await {
        log::error!("Client error: {}", e);
        exit(1);
    }

    let _ = network::shutdown_tls(client.messages()).await;
}

fn monitor_info(video: &sdl2::VideoSubsystem) -> Vec<MonitorInfo> {
    let displays = video.num_video_displays().unwrap_or(0);
    let mut monitors = Vec::new();
    for i in 0..displays {
        if let Ok(bounds) = video.display_bounds(i) {
            // x,y,w,h
            if let Ok(mode) = video.desktop_display_mode(i) {
                // refresh_rate, etc.
                monitors.push(MonitorInfo {
                    monitor_id: i as u32,
                    x: bounds.x(),
                    y: bounds.y(),
                    width: bounds.width(),
                    height: bounds.height(),
                    refresh_hz: mode.refresh_rate as u32,
                });
            } else {
                log::warn!("Failed to get display mode for monitor {}", i);
            }
        } else {
            log::warn!("Failed to get display bounds for monitor {}", i);
        }
    }
    monitors
}

fn default_window(host: String) -> WindowSettings {
    WindowSettings {
        window_id: 0,
        monitor_id: None,
        title: format!("GSH Client: {}", host),
        initial_mode: WindowMode::Windowed as i32,
        width: 800,
        height: 600,
        always_on_top: false,
        allow_resize: true,
        resize_frame: false,
        frame_anchor: window_settings::WindowAnchor::TopLeft as i32,
    }
}

fn create_id_file(name: String, id_files: &mut config::IdFiles) {
    let mut rng = OsRng;
    let bits = 2048;
    let private_key = RsaPrivateKey::new(&mut rng, bits).expect("Failed to generate a key");
    let public_key = RsaPublicKey::from(&private_key);

    let private_key_pem = private_key.to_pkcs8_pem().expect("Failed to encode private key");
    let public_key_pem = public_key.to_public_key_pem().expect("Failed to encode public key");

    let mut path = config::gsh_dir();
    path.push(format!("{}_{}.pem", name, rand::random::<u32>()));

    let mut file = File::create(&path).expect("Failed to create ID file");
    file.write_all(private_key_pem.as_bytes()).expect("Failed to write private key to file");
    file.write_all(public_key_pem.as_bytes()).expect("Failed to write public key to file");

    id_files.add_id_file(name, path);
}
