use env_logger::{init_from_env, Env};
use libgsh::{
    cert,
    rustls::ServerConfig,
    shared::{
        protocol::{frame_data::FrameFormat, window_settings, FrameData, WindowSettings},
        ClientEvent,
    },
    simple::{
        server::SimpleServer,
        service::{SimpleService, SimpleServiceExt},
    },
};
use log::trace;
use rand::random;
use std::sync::mpsc::{Receiver, Sender};

fn main() {
    init_from_env(Env::default().default_filter_or("info"));
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

pub struct ColorService {
    frames: Sender<FrameData>,
    events: Receiver<ClientEvent>,
    fill_color: (u8, u8, u8),
    changed_color: bool,
}

impl ColorService {
    fn new_frame(&self) -> FrameData {
        let format = FrameFormat::Rgba;
        let mut frame = [0; FRAME_WIDTH * FRAME_HEIGHT * 4];
        for i in 0..(FRAME_WIDTH * FRAME_HEIGHT) {
            frame[i * 4] = self.fill_color.0; // Red
            frame[i * 4 + 1] = self.fill_color.1; // Green
            frame[i * 4 + 2] = self.fill_color.2; // Blue
            frame[i * 4 + 3] = 255;
        }
        FrameData {
            format: format as i32,
            image_data: frame.to_vec(),
            width: FRAME_WIDTH as u32,
            height: FRAME_HEIGHT as u32,
        }
    }

    fn random_color() -> (u8, u8, u8) {
        let r = random::<u8>();
        let g = random::<u8>();
        let b = random::<u8>();
        (r, g, b)
    }
}

impl SimpleService for ColorService {
    fn new(frames: Sender<FrameData>, events: Receiver<ClientEvent>) -> Self {
        Self {
            frames,
            events,
            fill_color: Self::random_color(),
            changed_color: true,
        }
    }

    fn main(self) -> Result<(), Box<dyn std::error::Error>> {
        // We simply proxy to the `SimpleServiceExt` implementation.
        <Self as SimpleServiceExt>::main(self)
    }

    fn initial_window_settings() -> Option<WindowSettings> {
        Some(WindowSettings {
            id: 0,
            title: String::from("Colors!"),
            initial_mode: window_settings::WindowMode::Windowed as i32,
            width: FRAME_WIDTH as u32,
            height: FRAME_HEIGHT as u32,
            always_on_top: false,
            allow_resize: false,
        })
    }
}

// The `SimpleServiceExt` trait provides a default event loop implementation,
// we only need to implement the `events`, `tick` and `handle_event` methods.
impl SimpleServiceExt for ColorService {
    fn events(&self) -> &Receiver<ClientEvent> {
        &self.events
    }

    fn tick(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.changed_color {
            self.frames.send(self.new_frame())?;
            self.changed_color = false;
        }
        Ok(())
    }

    fn handle_event(&mut self, event: ClientEvent) -> Result<(), Box<dyn std::error::Error>> {
        if let ClientEvent::UserInput(input) = event {
            trace!("UserInput: {:?}", input);
            self.fill_color = Self::random_color();
            self.changed_color = true;
        }
        Ok(())
    }
}
