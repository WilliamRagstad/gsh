use auth::ClientAuthProvider;
use clap::{ColorChoice, CommandFactory, FromArgMatches, Parser, Subcommand};
use clap::builder::styling::{AnsiColor, Effects, Styles};
use client::Client;
use env_logger::fmt::WriteStyle;
use libgsh::{
    rsa::{pkcs1v15::VerifyingKey, signature::Verifier},
    sha2::Sha256,
    shared::{
        auth::AuthProvider,
        protocol::{
            client_hello::MonitorInfo,
            server_hello_ack::{
                window_settings::{self, WindowMode},
                FrameFormat, WindowSettings,
            },
        },
    },
};
use std::process::exit;

mod auth;
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
    /// The name of the ID file to use for authentication.
    #[clap(short, long)]
    id: Option<String>,
    /// Subcommand to execute.
    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a new named ID file
    NewId {
        /// The name of the ID file
        name: String,
    },
    /// List all known hosts
    ListHosts,
    /// List all IDs
    ListIds,
    /// Verify the ID files
    VerifyId {
        /// The name of the ID file
        name: String,
    },
}

#[tokio::main]
async fn main() {
    let color_choice = color_choice();

    // Show info logs by default so input events are visible during interactive use
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .write_style(match color_choice {
            ColorChoice::Always => WriteStyle::Always,
            ColorChoice::Never => WriteStyle::Never,
            ColorChoice::Auto => WriteStyle::Auto,
        })
        .format_line_number(true)
        .format_timestamp(None)
        .init();

    // Force colored help/errors by default (unless NO_COLOR / TERM=dumb).
    let mut cmd = Args::command();
    cmd = cmd.color(color_choice);
    cmd = cmd.styles(clap_styles());
    let matches = cmd.get_matches();
    let args = Args::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    let known_hosts = config::KnownHosts::load();
    let mut id_files = config::IdFiles::load();

    if let Some(command) = args.command {
        match command {
            Command::NewId { name } => {
                let path = id_files.create_id_file(&name);
                println!("ID file created at {} for {}", path.display(), name);
            }
            Command::ListHosts => {
                println!("Known hosts:");
                for host in known_hosts.hosts {
                    println!("Host: {}, Fingerprints: {:?}", host.host, host.fingerprints);
                }
            }
            Command::ListIds => {
                println!("ID files:");
                for (id_name, id_file) in id_files.files() {
                    println!("- {}: {}", id_name, id_file.display());
                }
            }
            Command::VerifyId { name } => {
                const MESSAGE: &[u8] = b"test";
                let mut provider = ClientAuthProvider::new(known_hosts, id_files, Some(name));
                match provider.signature("", MESSAGE) {
                    Some((signature, pub_key)) => {
                        log::trace!("Public key: {:?}", pub_key);
                        log::trace!("Signature: {:?}", signature);
                        let verifier_key = VerifyingKey::<Sha256>::new(pub_key);
                        if let Err(err) = verifier_key.verify(MESSAGE, &signature) {
                            log::error!("Signature verification failed: {}", err);
                            println!("Signature verification failed!");
                        } else {
                            println!("Successfully verified ID!");
                        }
                    }
                    None => {
                        println!("Invalid ID file or no public key found.");
                    }
                }
            }
        }
        return;
    }

    // Initialize SDL3
    let sdl = sdl3::init().unwrap_or_else(|e| {
        log::error!("Failed to initialize SDL3: {}", e);
        exit(1);
    });
    let video = sdl.video().unwrap_or_else(|e| {
        log::error!("Failed to initialize SDL3 video subsystem: {}", e);
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
        known_hosts,
        id_files,
        args.id,
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
    let compression = hello.compression;
    println!("Successfully connected to server!");

    let mut client = Client::new(sdl, video, format, compression, messages);

    if hello.windows.is_empty() {
        log::warn!("No initial window settings provided, creating a default window.");
        client
            .create_window(&default_window(&host))
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
    log::info!("Shutting down client...");
    let _ = network::shutdown_tls(client.inner_stream()).await;
}

fn color_choice() -> ColorChoice {
    if std::env::var_os("NO_COLOR").is_some() {
        return ColorChoice::Never;
    }
    if matches!(std::env::var("TERM").as_deref(), Ok("dumb")) {
        return ColorChoice::Never;
    }
    ColorChoice::Always
}

fn clap_styles() -> Styles {
    Styles::styled()
        .usage(
            AnsiColor::Yellow
                .on_default()
                .effects(Effects::BOLD | Effects::UNDERLINE),
        )
        .header(
            AnsiColor::Yellow
                .on_default()
                .effects(Effects::BOLD | Effects::UNDERLINE),
        )
        .literal(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .valid(
            AnsiColor::Green
                .on_default()
                .effects(Effects::BOLD | Effects::UNDERLINE),
        )
        .placeholder(AnsiColor::White.on_default())
}

fn monitor_info(video: &sdl3::VideoSubsystem) -> Vec<MonitorInfo> {
    let mut monitors = Vec::new();
    if let Ok(displays) = video.displays() {
        for (i, display) in displays.iter().enumerate() {
            // display.bounds() and display.desktop_mode() are SDL3 APIs exposed by the crate
            if let Ok(bounds) = display.get_bounds() {
                if let Ok(mode) = display.get_mode() {
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
    }
    monitors
}

fn default_window(host: &str) -> WindowSettings {
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
