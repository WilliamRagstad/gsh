use anyhow::{anyhow, Result};
use clap::Parser;
use network::Messages;
use sdl2::{
    event::{Event, WindowEvent},
    pixels::PixelFormatEnum,
    render::Canvas,
    video,
};
use shared::{
    prost::Message,
    protocol::{
        self,
        user_input::InputType,
        window_settings::{self, WindowMode},
        Frame, FrameFormat, UserInput, WindowSettings,
    },
};
use std::collections::HashMap;
// use window::WindowsMap;
// use window::WindowManager;

mod network;
// mod window;
// mod window;

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
    if let Err(e) = start(Args::parse()) {
        log::error!("Failed to start client: {}", e);
    }
}

fn start(args: Args) -> Result<()> {
    println!("Connecting to {}:{}...", args.host, args.port);
    let (windows, format, messages) = network::connect_tls(&args.host, args.port, args.insecure)?;
    println!("Successfully connected to server!");

    let mut client = Client::new(format, messages)?;

    if windows.is_empty() {
        log::warn!("No initial window settings provided, creating a default window.");
        // Add a default window if none are provided
        let default_window = WindowSettings {
            window_id: 0,
            title: format!("GSH Client: {}", args.host),
            initial_mode: WindowMode::Windowed as i32,
            width: 800,
            height: 600,
            always_on_top: false,
            allow_resize: true,
            resize_frame: false,
            anchor: window_settings::FrameAnchor::TopLeft as i32,
        };
        client.create_window(&default_window)?;
    } else {
        log::info!("Creating {} windows...", windows.len());
        for ws in windows {
            client.create_window(&ws)?;
        }
    }
    client.main_loop()?;

    let _ = network::shutdown_tls(client.messages());
    Ok(())
}

pub type WindowID = u32;

struct Client {
    sdl_context: sdl2::Sdl,
    video_subsystem: sdl2::VideoSubsystem,
    format: FrameFormat,
    /// Mapping from SDL2 window ID to SDL2 canvas video::Window
    windows: HashMap<WindowID, Canvas<video::Window>>,
    /// Mapping from server ID to SDL2 window ID
    server_window_to_sdl_window: HashMap<WindowID, WindowID>,
    sdl_window_to_server_window: HashMap<WindowID, WindowID>,
    messages: Messages,
}

impl Client {
    fn new(format: FrameFormat, messages: Messages) -> Result<Self> {
        let sdl_context = sdl2::init().map_err(|e| anyhow!(e))?;
        let video_subsystem = sdl_context.video().map_err(|e| anyhow!(e))?;
        Ok(Client {
            sdl_context,
            video_subsystem,
            format,
            windows: HashMap::new(),
            server_window_to_sdl_window: HashMap::new(),
            sdl_window_to_server_window: HashMap::new(),
            messages,
        })
    }

    fn messages(&mut self) -> &mut Messages {
        &mut self.messages
    }

    fn create_window(&mut self, ws: &WindowSettings) -> Result<WindowID> {
        let mut window = self.video_subsystem.window(&ws.title, ws.width, ws.height);
        window.position_centered();
        if ws.allow_resize {
            window.resizable();
        }
        if ws.initial_mode == WindowMode::Fullscreen as i32 {
            window.fullscreen();
        } else if ws.initial_mode == WindowMode::Borderless as i32 {
            window.borderless();
        } else if ws.initial_mode == WindowMode::WindowedMaximized as i32 {
            window.maximized();
        }
        let window = window.build().map_err(|e| anyhow!(e))?;
        let sdl_window_id = window.id();
        let mut canvas = window.into_canvas().build().map_err(|e| anyhow!(e))?;
        self.server_window_to_sdl_window
            .insert(ws.window_id, sdl_window_id);
        self.sdl_window_to_server_window
            .insert(sdl_window_id, ws.window_id);
        log::info!("Window ID {} created", ws.window_id);
        canvas.set_draw_color(sdl2::pixels::Color::BLACK);
        canvas.clear();
        canvas.present();
        self.windows.insert(sdl_window_id, canvas);
        Ok(ws.window_id)
    }

    fn destroy_window(&mut self, window_id: WindowID) -> Result<()> {
        if let Some(mut canvas) = self.windows.remove(&window_id) {
            canvas.window_mut().hide();
            self.messages.write_message(protocol::UserInput {
                kind: protocol::user_input::InputType::WindowClose as i32,
                window_id,
                key_code: 0,
                modifiers: 0,
                mouse_x: 0,
                mouse_y: 0,
                mouse_button: 0,
                scroll_delta: 0,
            })?;
            // Remove the window from the mapping
            if let Some(server_window_id) = self.sdl_window_to_server_window.remove(&window_id) {
                self.server_window_to_sdl_window.remove(&server_window_id);
            }
            log::info!("Window ID {} destroyed", window_id);
        } else {
            log::warn!("Window ID {} not found (not destroyed)", window_id);
        }
        Ok(())
    }

    fn get_format(&self) -> PixelFormatEnum {
        match self.format {
            FrameFormat::Rgba => PixelFormatEnum::RGBA32,
            FrameFormat::Rgb => PixelFormatEnum::RGB24,
        }
    }

    fn bytes_per_pixel(&self) -> usize {
        match self.format {
            FrameFormat::Rgba => 4,
            FrameFormat::Rgb => 3,
        }
    }

    fn main_loop(&mut self) -> Result<()> {
        // Set the socket to non-blocking mode
        // All calls to `read_message` will return immediately, even if no data is available
        self.messages.get_stream().sock.set_nonblocking(true)?;
        // Window event pump
        let mut event_pump = self.sdl_context.event_pump().map_err(|e| anyhow!(e))?;
        'running: loop {
            // Read messages from the server
            match self.messages.read_message() {
                Ok(buf) => {
                    if let Ok(frame) = protocol::Frame::decode(&buf[..]) {
                        self.render_frame(frame)?;
                    } else if let Ok(status_update) = protocol::StatusUpdate::decode(&buf[..]) {
                        if status_update.kind == protocol::status_update::StatusType::Exit as i32 {
                            log::trace!("Received graceful exit status, closing connection...");
                            break 'running;
                        } else {
                            log::trace!("StatusUpdate: {:?}", status_update);
                        }
                    } else {
                        log::trace!("Received data: {:?}", &buf[..]);
                        log::trace!("Unknown message type, ignoring...");
                    }
                }
                Err(err) => match err.kind() {
                    std::io::ErrorKind::WouldBlock => (), // No data available yet, do nothing
                    std::io::ErrorKind::UnexpectedEof => {
                        log::trace!("Server force disconnected, closing connection...");
                        break 'running;
                    }
                    _ => {
                        log::error!("Error reading message: {}", err);
                        break 'running;
                    }
                },
            };

            // Events from SDL2 windows
            for event in event_pump.poll_iter() {
                match event {
                    Event::Quit { .. } => {
                        log::trace!("Received quit event, exiting...");
                        break 'running;
                    }
                    Event::Window {
                        win_event: WindowEvent::Close,
                        window_id,
                        ..
                    } => self.destroy_window(window_id)?,
                    Event::KeyDown {
                        keycode: Some(key),
                        keymod,
                        window_id,
                        ..
                    } => {
                        log::trace!("Key pressed: {:?}", key);
                        // Send user input to server
                        self.messages.write_message(UserInput {
                            kind: InputType::KeyPress as i32,
                            key_code: key.into_i32(),
                            modifiers: keymod.bits() as u32,
                            scroll_delta: 0,
                            mouse_x: 0,
                            mouse_y: 0,
                            mouse_button: 0,
                            window_id: *self
                                .sdl_window_to_server_window
                                .get(&window_id)
                                .unwrap_or(&0),
                        })?;
                    }
                    _ => (),
                }
            }
        }
        log::trace!("Exiting main loop...");
        // Destroy all windows (Hacky way to ensure all windows are closed)
        let keys = self.windows.keys().cloned().collect::<Vec<_>>();
        for window_id in keys {
            self.destroy_window(window_id)?;
        }
        Ok(())
    }

    fn render_frame(&mut self, frame: Frame) -> Result<()> {
        let format = self.get_format();
        let pixel_bytes = self.bytes_per_pixel();
        let server_window_id = frame.window_id;
        if let Some(sdl_window_id) = self.server_window_to_sdl_window.get(&server_window_id) {
            log::trace!("Rendering frame for window ID {}", server_window_id);
            let canvas = self.windows.get_mut(sdl_window_id).unwrap();
            let texture_creator = canvas.texture_creator();
            let mut texture =
                texture_creator.create_texture_target(format, frame.width, frame.height)?;
            texture.update(None, &frame.data, frame.width as usize * pixel_bytes)?;
            canvas.copy(&texture, None, None).map_err(|e| anyhow!(e))?;
            canvas.present();
        } else {
            log::warn!(
                "Server Window ID {} not found in mapping (not rendered)",
                server_window_id
            );
        }
        Ok(())
    }
}
