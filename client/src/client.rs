use anyhow::{anyhow, Result};
use sdl2::{
    event::{Event, WindowEvent},
    pixels::PixelFormatEnum,
    rect::Rect,
    render::Canvas,
    video,
};
use shared::{
    prost::Message,
    protocol::{
        self,
        server_hello_ack::FrameFormat,
        user_input::{
            self, key_event::KeyAction, mouse_event::MouseAction, window_event::WindowAction,
            InputType,
        },
        window_settings::WindowMode,
        Frame, UserInput, WindowSettings,
    },
};
use std::collections::HashMap;

use crate::network::Messages;

const MAX_FPS: u32 = 60;
const FRAME_TIME: u64 = 1_000_000_000 / MAX_FPS as u64; // in nanoseconds
pub type WindowID = u32;

pub struct SdlWindow {
    // pub server_window_id: WindowID,
    // pub texture_creator: sdl2::render::TextureCreator<video::WindowContext>,
    // pub current_texture: sdl2::render::Texture<'static>,
    pub canvas: Canvas<video::Window>,
    // pub current_frame: Option<Frame>,
}

pub struct Client {
    sdl: sdl2::Sdl,
    video: sdl2::VideoSubsystem,
    format: FrameFormat,
    /// Mapping from SDL2 window ID to SDL2 canvas video::Window
    windows: HashMap<WindowID, SdlWindow>,
    /// Mapping from server ID to SDL2 window ID
    server_window_to_sdl_window: HashMap<WindowID, WindowID>,
    sdl_window_to_server_window: HashMap<WindowID, WindowID>,
    messages: Messages,
}

impl Client {
    pub fn new(
        sdl: sdl2::Sdl,
        video: sdl2::VideoSubsystem,
        format: FrameFormat,
        messages: Messages,
    ) -> Result<Self> {
        // let sdl_context = sdl2::init().map_err(|e| anyhow!(e))?;
        // let video_subsystem = sdl_context.video().map_err(|e| anyhow!(e))?;
        Ok(Client {
            sdl,
            video,
            format,
            windows: HashMap::new(),
            server_window_to_sdl_window: HashMap::new(),
            sdl_window_to_server_window: HashMap::new(),
            messages,
        })
    }

    pub fn messages(&mut self) -> &mut Messages {
        &mut self.messages
    }

    pub fn create_window(&mut self, ws: &WindowSettings) -> Result<WindowID> {
        let mut window = self.video.window(&ws.title, ws.width, ws.height);
        if let Some(monitor_id) = ws.monitor_id {
            let monitor = self
                .video
                .display_bounds(monitor_id as i32)
                .map_err(|e| anyhow!(e))?;
            let x = monitor.x() + ((monitor.width() as i32) - ws.width as i32) / 2;
            let y = monitor.y() + ((monitor.height() as i32) - ws.height as i32) / 2;
            window.position(x, y);
        } else {
            window.position_centered();
        }
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
        canvas.clear();
        canvas.present();
        let sdl_window = SdlWindow {
            // server_window_id: ws.window_id,
            canvas,
        };
        self.windows.insert(sdl_window_id, sdl_window);
        Ok(ws.window_id)
    }

    fn destroy_window(&mut self, window_id: WindowID) -> Result<()> {
        if let Some(mut win) = self.windows.remove(&window_id) {
            win.canvas.window_mut().hide();
            self.messages.write_message(protocol::UserInput {
                kind: protocol::user_input::InputType::WindowEvent as i32,
                window_id,
                input_event: Some(protocol::user_input::InputEvent::WindowEvent(
                    user_input::WindowEvent {
                        action: WindowAction::Close as i32,
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                    },
                )),
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

    fn key_event(
        &mut self,
        window_id: WindowID,
        action: KeyAction,
        keycode: sdl2::keyboard::Keycode,
        keymod: sdl2::keyboard::Mod,
    ) -> Result<()> {
        self.messages.write_message(UserInput {
            window_id: *self
                .sdl_window_to_server_window
                .get(&window_id)
                .unwrap_or(&0),
            kind: InputType::KeyEvent as i32,
            input_event: Some(user_input::InputEvent::KeyEvent(user_input::KeyEvent {
                action: action as i32,
                key_code: keycode.into_i32(),
                modifiers: keymod.bits() as u32,
            })),
        })?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn mouse_event(
        &mut self,
        window_id: WindowID,
        action: MouseAction,
        button: Option<sdl2::mouse::MouseButton>,
        mouse_x: i32,
        mouse_y: i32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<()> {
        let button = match button {
            Some(sdl2::mouse::MouseButton::Left) => {
                user_input::mouse_event::MouseButton::Left as i32
            }
            Some(sdl2::mouse::MouseButton::Middle) => {
                user_input::mouse_event::MouseButton::Middle as i32
            }
            Some(sdl2::mouse::MouseButton::Right) => {
                user_input::mouse_event::MouseButton::Right as i32
            }
            _ => 0,
        };

        self.messages.write_message(UserInput {
            window_id: *self
                .sdl_window_to_server_window
                .get(&window_id)
                .unwrap_or(&0),
            kind: InputType::MouseEvent as i32,
            input_event: Some(user_input::InputEvent::MouseEvent(user_input::MouseEvent {
                action: action as i32,
                x: mouse_x,
                y: mouse_y,
                button,
                delta_x,
                delta_y,
            })),
        })?;
        Ok(())
    }

    fn window_event(
        &mut self,
        window_id: WindowID,
        action: WindowAction,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<()> {
        self.messages.write_message(UserInput {
            window_id: *self
                .sdl_window_to_server_window
                .get(&window_id)
                .unwrap_or(&0),
            kind: InputType::WindowEvent as i32,
            input_event: Some(user_input::InputEvent::WindowEvent(
                user_input::WindowEvent {
                    action: action as i32,
                    x,
                    y,
                    width,
                    height,
                },
            )),
        })?;
        Ok(())
    }

    pub fn main(&mut self) -> Result<()> {
        // Set the socket to non-blocking mode
        // All calls to `read_message` will return immediately, even if no data is available
        self.messages.get_stream().sock.set_nonblocking(true)?;
        // Window event pump
        let mut event_pump = self.sdl.event_pump().map_err(|e| anyhow!(e))?;
        let mut last_frame_time = std::time::Instant::now();
        'running: loop {
            // Read messages from the server
            match self.messages.read_message() {
                Ok(buf) => {
                    if let Ok(frame) = protocol::Frame::decode(&buf[..]) {
                        self.render_frame(frame)?;
                    } else if let Ok(status_update) = protocol::StatusUpdate::decode(&buf[..]) {
                        if status_update.kind == protocol::status_update::StatusType::Exit as i32 {
                            log::trace!("Server gracefully disconnected!");
                            break 'running;
                        } else {
                            log::trace!("StatusUpdate: {:?}", status_update);
                        }
                    } else {
                        panic!("Failed to decode message: {:?}", buf);
                    }
                }
                Err(err) => match err.kind() {
                    std::io::ErrorKind::WouldBlock => (), // No data available yet, do nothing
                    std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::NotConnected => {
                        log::trace!("Server disconnected!");
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
                    } => {
                        self.window_event(window_id, WindowAction::Close, 0, 0, 0, 0)?;
                        log::trace!("Window {} closed", window_id);
                        self.destroy_window(window_id)?;
                    }
                    Event::Window {
                        win_event: WindowEvent::Resized(width, height),
                        window_id,
                        ..
                    } => {
                        self.window_event(
                            window_id,
                            WindowAction::Resize,
                            0,
                            0,
                            width as u32,
                            height as u32,
                        )?;
                        log::trace!("Window {} resized to {}x{}", window_id, width, height);
                    }
                    Event::Window {
                        win_event: WindowEvent::Moved(x, y),
                        window_id,
                        ..
                    } => {
                        self.window_event(window_id, WindowAction::Move, x, y, 0, 0)?;
                        log::trace!("Window {} moved to ({}, {})", window_id, x, y);
                    }
                    Event::KeyDown {
                        keycode: Some(keycode),
                        keymod,
                        window_id,
                        ..
                    } => self.key_event(window_id, KeyAction::Press, keycode, keymod)?,
                    Event::MouseMotion {
                        window_id, x, y, ..
                    } => {
                        self.mouse_event(window_id, MouseAction::Move, None, x, y, 0.0, 0.0)?;
                        log::trace!("Mouse moved in window {}: ({}, {})", window_id, x, y);
                    }
                    Event::MouseButtonDown {
                        window_id,
                        mouse_btn,
                        x,
                        y,
                        ..
                    } => {
                        self.mouse_event(
                            window_id,
                            MouseAction::Press,
                            Some(mouse_btn),
                            x,
                            y,
                            0.0,
                            0.0,
                        )?;
                        log::trace!(
                            "Mouse button pressed in window {}: ({}, {})",
                            window_id,
                            x,
                            y
                        );
                    }
                    Event::MouseButtonUp {
                        window_id,
                        mouse_btn,
                        x,
                        y,
                        ..
                    } => {
                        self.mouse_event(
                            window_id,
                            MouseAction::Release,
                            Some(mouse_btn),
                            x,
                            y,
                            0.0,
                            0.0,
                        )?;
                        log::trace!(
                            "Mouse button released in window {}: ({}, {})",
                            window_id,
                            x,
                            y
                        );
                    }
                    Event::MouseWheel {
                        window_id,
                        direction: _direction,
                        precise_x,
                        precise_y,
                        mouse_x,
                        mouse_y,
                        ..
                    } => {
                        self.mouse_event(
                            window_id,
                            MouseAction::Scroll,
                            None,
                            mouse_x,
                            mouse_y,
                            precise_x,
                            precise_y,
                        )?;
                        log::trace!(
                            "Mouse wheel scrolled in window {}: ({}, {})",
                            window_id,
                            mouse_x,
                            mouse_y,
                        );
                    }
                    _ => {
                        log::trace!("Unhandled event: {:?}", event);
                    }
                }
            }

            // Sleep to maintain frame rate
            let elapsed_time = last_frame_time.elapsed().as_nanos() as u64;
            if elapsed_time < FRAME_TIME {
                std::thread::sleep(std::time::Duration::new(
                    0,
                    (FRAME_TIME - elapsed_time) as u32,
                ));
            }
            last_frame_time = std::time::Instant::now();
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
        if frame.segments.is_empty() || frame.width == 0 || frame.height == 0 {
            log::warn!("Received empty frame, skipping rendering.");
            return Ok(());
        }
        let format = self.get_format();
        let pixel_bytes = self.bytes_per_pixel();
        let server_window_id = frame.window_id;
        if let Some(sdl_window_id) = self.server_window_to_sdl_window.get(&server_window_id) {
            log::trace!(
                "Rendering frame ({} segments) for window ID {}",
                frame.segments.len(),
                server_window_id
            );
            let win = self.windows.get_mut(sdl_window_id).unwrap();
            let texture_creator = win.canvas.texture_creator();
            let mut texture =
                texture_creator.create_texture_target(format, frame.width, frame.height)?;
            // Apply all segments of the frame to the window
            for segment in &frame.segments {
                if segment.width == 0 || segment.height == 0 {
                    log::warn!("Received empty segment, skipping rendering.");
                    continue;
                }
                texture.update(
                    Some(Rect::new(
                        segment.x,
                        segment.y,
                        segment.width,
                        segment.height,
                    )),
                    &segment.data,
                    frame.width as usize * pixel_bytes,
                )?;
            }
            win.canvas
                .copy(&texture, None, None)
                .map_err(|e| anyhow!(e))?;
            win.canvas.present();
        } else {
            log::warn!(
                "Server Window ID {} not found in mapping (not rendered)",
                server_window_id
            );
        }
        Ok(())
    }
}
