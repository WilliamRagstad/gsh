use std::process::exit;

use clap::Parser;
use client::Client;
use shared::protocol::{
    client_hello::MonitorInfo,
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_line_number(true)
        .format_timestamp(None)
        .init();
    let args = Args::parse();

    // Initialize SDL2
    let sdl = sdl2::init().unwrap_or_else(|e| {
        log::error!("Failed to initialize SDL2: {}", e);
        exit(1);
    });
    let video = sdl.video().unwrap_or_else(|e| {
        log::error!("Failed to initialize SDL2 video subsystem: {}", e);
        exit(1);
    });

    println!("Connecting to {}:{}...", args.host, args.port);
    let (windows, format, messages) =
        network::connect_tls(&args.host, args.port, args.insecure, monitor_info(&video))
            .unwrap_or_else(|e| {
                log::error!("Failed to connect: {}", e);
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
