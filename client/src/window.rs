use anyhow::Result;
use sdl2::rect::Rect;
use sdl2::{pixels::Color, render};
use shared::protocol::{frame_data::FrameFormat, FrameData, UserInput};
use shared::protocol::{user_input, window_settings, WindowSettings};
use shared::ClientEvent;

/// SDL2 Window management, event handling and message passing to protocol channel
pub struct ClientWindow {
    _sdl_context: sdl2::Sdl,
    _video_subsystem: sdl2::VideoSubsystem,
    // window: sdl2::video::Window,
    canvas: render::Canvas<sdl2::video::Window>,
    event_pump: sdl2::EventPump,
    server_sender: std::sync::mpsc::Sender<ClientEvent>,
    server_receiver: std::sync::mpsc::Receiver<FrameData>,
}

impl ClientWindow {
    const DEAFULT_WIDTH: u32 = 800;
    const DEAFULT_HEIGHT: u32 = 600;
    const DEFAULT_TITLE_PREFIX: &'static str = "GSH Client";
    const DEFAULT_ALLOW_RESIZE: bool = true;

    fn load_settings(
        initial_window_settings: Option<WindowSettings>,
        host: String,
    ) -> WindowSettings {
        if let Some(iws) = initial_window_settings {
            iws
        } else {
            WindowSettings {
                id: 0,
                title: format!("{} {}", Self::DEFAULT_TITLE_PREFIX, host),
                initial_mode: window_settings::WindowMode::Windowed as i32,
                width: Self::DEAFULT_WIDTH,
                height: Self::DEAFULT_HEIGHT,
                always_on_top: false,
                allow_resize: Self::DEFAULT_ALLOW_RESIZE,
            }
        }
    }

    pub fn new(
        server_sender: std::sync::mpsc::Sender<ClientEvent>,
        server_receiver: std::sync::mpsc::Receiver<FrameData>,
        initial_window_settings: Option<WindowSettings>,
        host: String,
    ) -> Self {
        let iws = Self::load_settings(initial_window_settings, host);
        let sdl_context = sdl2::init().unwrap();
        let video_subsystem = sdl_context.video().unwrap();
        let mut window = video_subsystem.window(&iws.title, iws.width, iws.height);
        window.position_centered();
        if iws.allow_resize {
            window.resizable();
        }
        if iws.initial_mode == window_settings::WindowMode::Fullscreen as i32 {
            window.fullscreen();
        } else if iws.initial_mode == window_settings::WindowMode::Borderless as i32 {
            window.borderless();
        } else if iws.initial_mode == window_settings::WindowMode::WindowedMaximized as i32 {
            window.maximized();
        }
        let window = window.build().unwrap_or_else(|err| {
            panic!("Failed to create window: {}", err);
        });

        let canvas = window.into_canvas().build().unwrap();
        let event_pump = sdl_context.event_pump().unwrap();

        Self {
            _sdl_context: sdl_context,
            _video_subsystem: video_subsystem,
            canvas,
            event_pump,
            server_sender,
            server_receiver,
        }
    }

    fn render_frame(&mut self, frame: &FrameData) -> Result<()> {
        // Here you would typically update the window with the new frame data
        // For example, using SDL2 to create a texture and render it.
        // Here you would typically create a texture from the frame data and render it to the window
        // For example:
        // let texture_creator = self.window.texture_creator();
        // let texture = texture_creator.create_texture_from_surface(&frame.image_data)?;
        // self.window.copy(&texture, None, None)?;
        // log::trace!("Received frame data: {:?}", frame);
        if frame.format != FrameFormat::Rgba as i32 {
            return Err(anyhow::anyhow!("Unsupported frame format"));
        }
        self.canvas.set_draw_color(Color::BLACK);
        self.canvas.clear();

        // Draw frame data as texture
        let texture_creator = self.canvas.texture_creator();
        let mut texture = texture_creator
            .create_texture_target(
                texture_creator.default_pixel_format(),
                frame.width,
                frame.height,
            )
            .unwrap();
        texture.update(None, &frame.image_data, frame.width as usize * 4)?; // Assuming RGBA format
        self.canvas
            .copy(
                &texture,
                None,
                Some(Rect::new(0, 0, frame.width, frame.height)),
            )
            .map_err(anyhow::Error::msg)?;

        // Update the window with the new frame data
        self.canvas.present();
        Ok(())
    }

    pub fn main(mut self) -> Result<()> {
        log::trace!("SDL2 Window started...");
        loop {
            match self.server_receiver.try_recv() {
                Ok(frame) => {
                    if let Err(e) = self.render_frame(&frame) {
                        log::trace!("Error rendering frame: {}", e);
                    }
                }
                Err(e) => match e {
                    std::sync::mpsc::TryRecvError::Empty => (), // do nothing, just continue
                    std::sync::mpsc::TryRecvError::Disconnected => {
                        log::trace!("Server disconnected, exiting...");
                        break;
                    }
                },
            }
            for event in self.event_pump.poll_iter() {
                match event {
                    sdl2::event::Event::Quit { .. } => {
                        log::trace!("Received Quit event, exiting...");
                        return Ok(());
                    }
                    sdl2::event::Event::KeyDown {
                        keycode: Some(key),
                        keymod,
                        ..
                    } => {
                        log::trace!("Key pressed: {:?}", key);
                        // Send user input to server
                        self.server_sender.send(ClientEvent::UserInput(UserInput {
                            kind: user_input::InputType::KeyPress as i32,
                            key_code: key.into_i32(),
                            modifiers: keymod.bits() as u32,
                            delta: 0,
                            x: 0,
                            y: 0,
                        }))?;
                    }
                    _ => (),
                }
            }
            // Sleep for a short duration to avoid busy waiting
            std::thread::sleep(std::time::Duration::from_millis(16)); // ~60 FPS
        }
        drop(self.server_sender);
        drop(self.server_receiver);
        log::trace!("SDL2 Window closed.");
        Ok(())
    }
}
