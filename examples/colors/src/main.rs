use env_logger::Env;
use libgsh::{
    cert,
    frame::optimize_segments,
    shared::{
        protocol::{
            server_hello_ack::{window_settings, FrameFormat, WindowSettings},
            Frame, ServerHelloAck,
        },
        ClientEvent,
    },
    simple::{
        server::SimpleServer,
        service::{SimpleService, SimpleServiceExt},
        Messages,
    },
    tokio_rustls::rustls::ServerConfig,
    Result,
};
use log::trace;
use rand::random;

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_line_number(true)
        .format_file(true)
        .format_target(false)
        .format_timestamp(None)
        .init();
    let (key, private_key) = cert::self_signed(&["localhost"]).unwrap();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![key.cert.der().clone()], private_key)
        .unwrap();
    let server: SimpleServer<ColorService> = SimpleServer::new(config);
    server.serve().unwrap();
}

const FRAME_WIDTH: usize = 250;
const FRAME_HEIGHT: usize = 250;
const PIXEL_BYTES: usize = 4; // RGBA
const WINDOW_PRIMARY: u32 = 0;
const WINDOW_SECONDARY: u32 = 1;

type Color = (u8, u8, u8);

pub struct ColorService {
    color: Color,
    prev_frame: Vec<u8>,
}

impl ColorService {
    fn send_frame(&mut self, messages: &mut Messages, window_id: u32, color: Color) -> Result<()> {
        let mut frame = [0; FRAME_WIDTH * FRAME_HEIGHT * PIXEL_BYTES];
        for i in 0..(FRAME_WIDTH * FRAME_HEIGHT) {
            frame[i * PIXEL_BYTES] = color.0; // Red
            frame[i * PIXEL_BYTES + 1] = color.1; // Green
            frame[i * PIXEL_BYTES + 2] = color.2; // Blue
            frame[i * PIXEL_BYTES + 3] = 255;
        }
        messages.write_message(Frame {
            window_id,
            // data: frame.to_vec(),
            segments: optimize_segments(
                &frame,
                FRAME_WIDTH,
                FRAME_HEIGHT,
                &mut self.prev_frame,
                PIXEL_BYTES,
            ),
            width: FRAME_WIDTH as u32,
            height: FRAME_HEIGHT as u32,
        })?;
        Ok(())
    }

    fn random_color() -> (u8, u8, u8) {
        let r = random::<u8>();
        let g = random::<u8>();
        let b = random::<u8>();
        (r, g, b)
    }

    fn swap_colors(&mut self, messages: &mut Messages) -> Result<()> {
        self.send_frame(messages, WINDOW_SECONDARY, self.color)?;
        self.color = Self::random_color();
        self.send_frame(messages, WINDOW_PRIMARY, self.color)?;
        Ok(())
    }
}

impl SimpleService for ColorService {
    fn new() -> Self {
        Self {
            color: Color::default(),
            prev_frame: Vec::new(),
        }
    }

    fn main(self, messages: Messages) -> Result<()> {
        // We simply proxy to the `SimpleServiceExt` implementation.
        <Self as SimpleServiceExt>::main(self, messages)
    }

    fn server_hello() -> ServerHelloAck {
        ServerHelloAck {
            format: FrameFormat::Rgba.into(),
            windows: vec![
                WindowSettings {
                    window_id: WINDOW_PRIMARY,
                    monitor_id: None,
                    title: String::from("Colors!"),
                    initial_mode: window_settings::WindowMode::Windowed.into(),
                    width: FRAME_WIDTH as u32,
                    height: FRAME_HEIGHT as u32,
                    always_on_top: false,
                    allow_resize: false,
                    resize_frame: false,
                    frame_anchor: window_settings::WindowAnchor::Center.into(),
                },
                WindowSettings {
                    window_id: WINDOW_SECONDARY,
                    monitor_id: None,
                    title: String::from("Previous"),
                    initial_mode: window_settings::WindowMode::Windowed.into(),
                    width: FRAME_WIDTH as u32,
                    height: FRAME_HEIGHT as u32,
                    always_on_top: false,
                    allow_resize: false,
                    resize_frame: false,
                    frame_anchor: window_settings::WindowAnchor::Center.into(),
                },
            ],
            auth_method: None,
        }
    }
}

// The `SimpleServiceExt` trait provides a default event loop implementation,
// we only need to implement the `events`, `tick` and `handle_event` methods.
impl SimpleServiceExt for ColorService {
    fn on_startup(&mut self, messages: &mut Messages) -> Result<()> {
        self.swap_colors(messages)
    }

    fn on_event(&mut self, messages: &mut Messages, event: ClientEvent) -> Result<()> {
        if let ClientEvent::UserInput(input) = event {
            trace!("UserInput: {:?}", input);
            self.swap_colors(messages)?;
        }
        Ok(())
    }
}
