use anyhow::Result;
use sdl2::rect::Rect;
use sdl2::{pixels::Color, render};
use shared::protocol::{user_input, window_settings, WindowSettings};
use shared::protocol::{Frame, UserInput};
use shared::ClientEvent;
use std::collections::HashMap;
use std::sync::mpsc;

pub type WindowsMap = HashMap<u32, WindowHnd>;
pub struct WindowHnd {
    // wnd: window::ClientWindow,
    pub thread: std::thread::JoinHandle<()>,
    pub event_recv: mpsc::Receiver<ClientEvent>,
    pub frame_send: mpsc::Sender<Frame>,
}

/// SDL2 Window management, event handling and message passing to protocol channel
pub struct ClientWindow {
    settings: WindowSettings,
    // _sdl_context: sdl2::Sdl,
    // video_subsystem: sdl2::VideoSubsystem,
    // window: sdl2::video::Window,
    canvas: render::Canvas<sdl2::video::Window>,
    event_pump: sdl2::EventPump,
    server_sender: mpsc::Sender<ClientEvent>,
    server_receiver: mpsc::Receiver<Frame>,
}

impl ClientWindow {
    pub const DEFAULT_WINDOW_ID: u32 = 0;
    const DEAFULT_WIDTH: u32 = 800;
    const DEAFULT_HEIGHT: u32 = 600;
    const DEFAULT_TITLE_PREFIX: &'static str = "GSH Client";
    const DEFAULT_ALLOW_RESIZE: bool = true;

    fn default_settings(host: String) -> WindowSettings {
        WindowSettings {
            window_id: Self::DEFAULT_WINDOW_ID,
            title: format!("{} {}", Self::DEFAULT_TITLE_PREFIX, host),
            initial_mode: window_settings::WindowMode::Windowed as i32,
            width: Self::DEAFULT_WIDTH,
            height: Self::DEAFULT_HEIGHT,
            always_on_top: false,
            allow_resize: Self::DEFAULT_ALLOW_RESIZE,
            resize_frame: false,
            anchor: window_settings::FrameAnchor::TopLeft as i32,
        }
    }

    pub fn default_new(
        server_sender: mpsc::Sender<ClientEvent>,
        server_receiver: mpsc::Receiver<Frame>,
        host: String,
        sdl_context: &sdl2::Sdl,
    ) -> Self {
        Self::new(
            server_sender,
            server_receiver,
            Self::default_settings(host),
            sdl_context,
        )
    }

    pub fn new(
        server_sender: mpsc::Sender<ClientEvent>,
        server_receiver: mpsc::Receiver<Frame>,
        settings: WindowSettings,
        sdl_context: &sdl2::Sdl,
    ) -> Self {
        // let sdl_context = sdl();
        let video_subsystem = sdl_context.video().unwrap();
        let mut window = video_subsystem.window(&settings.title, settings.width, settings.height);
        window.position_centered();
        if settings.allow_resize {
            window.resizable();
        }
        if settings.initial_mode == window_settings::WindowMode::Fullscreen as i32 {
            window.fullscreen();
        } else if settings.initial_mode == window_settings::WindowMode::Borderless as i32 {
            window.borderless();
        } else if settings.initial_mode == window_settings::WindowMode::WindowedMaximized as i32 {
            window.maximized();
        }
        let window = window.build().unwrap_or_else(|err| {
            panic!("Failed to create window: {}", err);
        });

        let canvas = window.into_canvas().build().unwrap();
        let event_pump = sdl_context.event_pump().unwrap();

        Self {
            settings,
            // video_subsystem,
            canvas,
            event_pump,
            server_sender,
            server_receiver,
        }
    }

    fn render_frame(&mut self, frame: &Frame) -> Result<()> {
        // Here you would typically update the window with the new frame data
        // For example, using SDL2 to create a texture and render it.
        // Here you would typically create a texture from the frame data and render it to the window
        // For example:
        // let texture_creator = self.window.texture_creator();
        // let texture = texture_creator.create_texture_from_surface(&frame.image_data)?;
        // self.window.copy(&texture, None, None)?;
        // log::trace!("Received frame data: {:?}", frame);
        // if frame.format != FrameFormat::Rgba as i32 {
        //     return Err(anyhow::anyhow!("Unsupported frame format"));
        // }
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
        texture.update(None, &frame.data, frame.width as usize * 4)?; // Assuming RGBA format
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
                    mpsc::TryRecvError::Empty => (), // do nothing, just continue
                    mpsc::TryRecvError::Disconnected => {
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
                            scroll_delta: 0,
                            mouse_x: 0,
                            mouse_y: 0,
                            mouse_button: 0,
                            window_id: self.settings.window_id,
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
