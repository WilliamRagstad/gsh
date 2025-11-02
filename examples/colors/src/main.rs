use env_logger::Env;
use libgsh::{
    async_trait::async_trait,
    server::{GshServer, GshService, GshServiceExt, GshStream},
    shared::{
        cert,
        frame::optimize_segments,
        protocol::{
            client_message::ClientEvent,
            server_hello_ack::{window_settings, FrameFormat, WindowSettings},
            Frame, ServerHelloAck,
        },
    },
    tokio, Result, ServerConfig,
};
use log::trace;
use rand::random;

#[tokio::main]
async fn main() {
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
    let server: GshServer<ColorService> = GshServer::new(ColorService::default(), config);
    server.serve().await.unwrap();
}

const FRAME_WIDTH: usize = 250;
const FRAME_HEIGHT: usize = 250;
const PIXEL_BYTES: usize = 4; // RGBA
const WINDOW_PRIMARY: u32 = 0;
const WINDOW_SECONDARY: u32 = 1;

type Color = (u8, u8, u8);

#[derive(Debug, Clone, Default)]
pub struct ColorService {
    color: Color,
    prev_frame: Vec<u8>,
}

impl ColorService {
    async fn send_frame(
        &mut self,
        stream: &mut GshStream,
        window_id: u32,
        color: Color,
    ) -> Result<()> {
        let mut frame = [0; FRAME_WIDTH * FRAME_HEIGHT * PIXEL_BYTES];
        for i in 0..(FRAME_WIDTH * FRAME_HEIGHT) {
            frame[i * PIXEL_BYTES] = color.0; // Red
            frame[i * PIXEL_BYTES + 1] = color.1; // Green
            frame[i * PIXEL_BYTES + 2] = color.2; // Blue
            frame[i * PIXEL_BYTES + 3] = 255;
        }
        stream
            .send(Frame {
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
            })
            .await?;
        Ok(())
    }

    fn random_color() -> (u8, u8, u8) {
        let r = random::<u8>();
        let g = random::<u8>();
        let b = random::<u8>();
        (r, g, b)
    }

    async fn swap_colors(&mut self, stream: &mut GshStream) -> Result<()> {
        self.send_frame(stream, WINDOW_SECONDARY, self.color)
            .await?;
        self.color = Self::random_color();
        self.send_frame(stream, WINDOW_PRIMARY, self.color).await?;
        Ok(())
    }
}

#[async_trait]
impl GshService for ColorService {
    async fn main(self, stream: GshStream) -> libgsh::Result<()> {
        <Self as GshServiceExt>::main(self, stream).await
    }

    fn server_hello(&self) -> ServerHelloAck {
        ServerHelloAck {
            format: FrameFormat::Rgba.into(),
            compression: None,
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

// The `GshServiceExt` trait provides a default event loop implementation,
// we only need to implement the `events`, `tick` and `handle_event` methods.
#[async_trait]
impl GshServiceExt for ColorService {
    async fn on_startup(&mut self, stream: &mut GshStream) -> Result<()> {
        self.swap_colors(stream).await
    }

    async fn on_event(&mut self, stream: &mut GshStream, event: ClientEvent) -> Result<()> {
        if let ClientEvent::UserInput(input) = event {
            trace!("UserInput: {:?}", input);
            self.swap_colors(stream).await?;
        }
        Ok(())
    }
}
