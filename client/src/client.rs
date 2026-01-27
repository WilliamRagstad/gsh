use anyhow::{anyhow, Result};
use libgsh::{
    client::GshStream,
    shared::protocol::{
        self,
        server_hello_ack::{self, window_settings::WindowMode, FrameFormat, WindowSettings},
        server_message::ServerEvent,
        status_update::{Details, StatusType},
        user_input::{
            self, key_event::KeyAction, mouse_event::MouseAction, window_event::WindowAction,
            InputType,
        },
        Frame, StatusUpdate, UserInput,
    },
};
use sdl3::{
    event::{Event, WindowEvent},
    pixels::{Color, PixelFormat},
    rect::Rect,
    render::{BlendMode, Canvas},
    video,
};
use std::{
    collections::HashMap,
    io::Read,
    time::{Duration, Instant},
};

const MAX_FPS: u32 = 60;
const FRAME_TIME: u64 = 1_000_000_000 / MAX_FPS as u64; // in nanoseconds
pub type WindowID = u32;

pub struct SdlWindow {
    // pub server_window_id: WindowID,
    // pub texture_creator: sdl3::render::TextureCreator<video::WindowContext>,
    // pub current_texture: sdl3::render::Texture<'static>,
    pub canvas: Canvas<video::Window>,
    // pub current_frame: Option<Frame>,
}

pub struct Client {
    sdl: sdl3::Sdl,
    video: sdl3::VideoSubsystem,
    format: FrameFormat,
    compression: Option<protocol::server_hello_ack::Compression>,
    /// Mapping from SDL window ID to SDL canvas video::Window
    windows: HashMap<WindowID, SdlWindow>,
    /// Mapping from server ID to SDL window ID
    server_window_to_sdl_window: HashMap<WindowID, WindowID>,
    sdl_window_to_server_window: HashMap<WindowID, WindowID>,
    stream: GshStream,
}

impl Client {
    pub fn new(
        sdl: sdl3::Sdl,
        video: sdl3::VideoSubsystem,
        format: FrameFormat,
        compression: Option<protocol::server_hello_ack::Compression>,
        stream: GshStream,
    ) -> Self {
        Client {
            sdl,
            video,
            format,
            compression,
            windows: HashMap::new(),
            server_window_to_sdl_window: HashMap::new(),
            sdl_window_to_server_window: HashMap::new(),
            stream,
        }
    }

    pub fn inner_stream(&mut self) -> &mut GshStream {
        &mut self.stream
    }

    pub fn create_window(&mut self, ws: &WindowSettings) -> Result<WindowID> {
        let mut window = self.video.window(&ws.title, ws.width, ws.height);
        if let Some(monitor_id) = ws.monitor_id {
            // SDL3 exposes displays() which returns a Vec<Display>; use it to get bounds
            if let Ok(displays) = self.video.displays() {
                if let Some(display) = displays.get(monitor_id as usize) {
                    if let Ok(bounds) = display.get_bounds() {
                        let x = bounds.x() + ((bounds.width() as i32) - ws.width as i32) / 2;
                        let y = bounds.y() + ((bounds.height() as i32) - ws.height as i32) / 2;
                        window.position(x, y);
                    }
                }
            }
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
        // SDL3's into_canvas API returns a Canvas directly
        let mut canvas = window.into_canvas();
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

    async fn destroy_window(&mut self, window_id: WindowID) -> Result<()> {
        if let Some(mut win) = self.windows.remove(&window_id) {
            win.canvas.window_mut().hide();
            // Translate SDL window id to server window id if possible
            if let Some(server_window_id) = self.sdl_window_to_server_window.remove(&window_id) {
                // Remove reverse mapping
                self.server_window_to_sdl_window.remove(&server_window_id);
                self.stream
                    .send(protocol::UserInput {
                        window_id: server_window_id,
                        kind: protocol::user_input::InputType::WindowEvent as i32,
                        input_event: Some(protocol::user_input::InputEvent::WindowEvent(
                            user_input::WindowEvent {
                                action: WindowAction::Close as i32,
                                x: 0,
                                y: 0,
                                width: 0,
                                height: 0,
                            },
                        )),
                    })
                    .await?;
                log::info!(
                    "Window ID {} destroyed (server id {})",
                    window_id,
                    server_window_id
                );
            } else {
                // Fallback: send to window 0 if no mapping exists
                self.stream
                    .send(protocol::UserInput {
                        window_id: 0,
                        kind: protocol::user_input::InputType::WindowEvent as i32,
                        input_event: Some(protocol::user_input::InputEvent::WindowEvent(
                            user_input::WindowEvent {
                                action: WindowAction::Close as i32,
                                x: 0,
                                y: 0,
                                width: 0,
                                height: 0,
                            },
                        )),
                    })
                    .await?;
                log::info!("Window ID {} destroyed (no server mapping)", window_id);
            }
        } else {
            log::warn!("Window ID {} not found (not destroyed)", window_id);
        }
        Ok(())
    }

    fn get_format(&self) -> PixelFormat {
        match self.format {
            FrameFormat::Rgba => PixelFormat::RGBA32,
            FrameFormat::Rgb => PixelFormat::RGB24,
        }
    }

    fn bytes_per_pixel(&self) -> usize {
        match self.format {
            FrameFormat::Rgba => 4,
            FrameFormat::Rgb => 3,
        }
    }

    async fn key_event(
        &mut self,
        window_id: WindowID,
        action: KeyAction,
        keycode: sdl3::keyboard::Keycode,
        keymod: sdl3::keyboard::Mod,
    ) -> Result<()> {
        self.stream
            .send(UserInput {
                window_id: *self
                    .sdl_window_to_server_window
                    .get(&window_id)
                    .unwrap_or(&0),
                kind: InputType::KeyEvent as i32,
                input_event: Some(user_input::InputEvent::KeyEvent(user_input::KeyEvent {
                    action: action as i32,
                    key_code: keycode as i32,
                    modifiers: keymod.bits() as u32,
                })),
            })
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn mouse_event(
        &mut self,
        window_id: WindowID,
        action: MouseAction,
        button: Option<sdl3::mouse::MouseButton>,
        mouse_x: i32,
        mouse_y: i32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<()> {
        let button = match button {
            Some(sdl3::mouse::MouseButton::Left) => {
                user_input::mouse_event::MouseButton::Left as i32
            }
            Some(sdl3::mouse::MouseButton::Middle) => {
                user_input::mouse_event::MouseButton::Middle as i32
            }
            Some(sdl3::mouse::MouseButton::Right) => {
                user_input::mouse_event::MouseButton::Right as i32
            }
            _ => 0,
        };

        let server_window_id = *self
            .sdl_window_to_server_window
            .get(&window_id)
            .unwrap_or(&0);
        log::trace!(
            "Sending mouse event -> server_window_id={}, action={:?}, x={}, y={}, button={}, dx={}, dy={}",
            server_window_id,
            action,
            mouse_x,
            mouse_y,
            button,
            delta_x,
            delta_y
        );
        self.stream
            .send(UserInput {
                window_id: server_window_id,
                kind: InputType::MouseEvent as i32,
                input_event: Some(user_input::InputEvent::MouseEvent(user_input::MouseEvent {
                    action: action as i32,
                    x: mouse_x,
                    y: mouse_y,
                    button,
                    delta_x,
                    delta_y,
                })),
            })
            .await?;
        Ok(())
    }

    async fn window_event(
        &mut self,
        window_id: WindowID,
        action: WindowAction,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<()> {
        self.stream
            .send(UserInput {
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
            })
            .await?;
        Ok(())
    }

    async fn handle_window_event(&mut self, event: Event) -> Result<bool> {
        log::trace!("SDL event: {:?}", event);
        match event {
            Event::Quit { .. } => {
                log::trace!("Received quit event, exiting...");
                return Ok(false);
            }
            Event::Window {
                win_event,
                window_id,
                ..
            } => {
                // SDL3's WindowEvent variants may differ; handle common ones and
                // fall back to checking the debug string for Close-type events.
                if win_event == WindowEvent::CloseRequested {
                    self.window_event(window_id, WindowAction::Close, 0, 0, 0, 0)
                        .await?;
                    log::trace!("Window {} closed", window_id);
                    self.destroy_window(window_id).await?;
                } else if let WindowEvent::Resized(width, height) = win_event {
                    self.window_event(
                        window_id,
                        WindowAction::Resize,
                        0,
                        0,
                        width as u32,
                        height as u32,
                    )
                    .await?;
                    log::trace!("Window {} resized to {}x{}", window_id, width, height);
                } else if let WindowEvent::Moved(x, y) = win_event {
                    self.window_event(window_id, WindowAction::Move, x, y, 0, 0)
                        .await?;
                    log::trace!("Window {} moved to ({}, {})", window_id, x, y);
                } else if win_event == WindowEvent::MouseEnter {
                    // Mouse entered the window (fallback via debug string)
                    self.mouse_event(window_id, MouseAction::Enter, None, 0, 0, 0.0, 0.0)
                        .await?;
                    log::trace!("Mouse entered window {}", window_id);
                } else if win_event == WindowEvent::MouseLeave {
                    // Mouse left the window (fallback via debug string)
                    self.mouse_event(window_id, MouseAction::Exit, None, 0, 0, 0.0, 0.0)
                        .await?;
                    log::trace!("Mouse left window {}", window_id);
                }
            }
            Event::KeyDown {
                keycode: Some(keycode),
                keymod,
                window_id,
                ..
            } => {
                self.key_event(window_id, KeyAction::Press, keycode, keymod)
                    .await?
            }
            Event::KeyUp {
                keycode: Some(keycode),
                keymod,
                window_id,
                ..
            } => {
                self.key_event(window_id, KeyAction::Release, keycode, keymod)
                    .await?
            }
            Event::MouseMotion {
                window_id, x, y, ..
            } => {
                self.mouse_event(
                    window_id,
                    MouseAction::Move,
                    None,
                    x as i32,
                    y as i32,
                    0.0,
                    0.0,
                )
                .await?;
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
                    x as i32,
                    y as i32,
                    0.0,
                    0.0,
                )
                .await?;
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
                    x as i32,
                    y as i32,
                    0.0,
                    0.0,
                )
                .await?;
                log::trace!(
                    "Mouse button released in window {}: ({}, {})",
                    window_id,
                    x,
                    y
                );
            }
            Event::MouseWheel {
                window_id, x, y, ..
            } => {
                // SDL3 MouseWheel fields differ; use x/y as deltas. Position may not be available.
                self.mouse_event(
                    window_id,
                    MouseAction::Scroll,
                    None,
                    0,
                    0,
                    x as f32,
                    y as f32,
                )
                .await?;
                log::trace!(
                    "Mouse wheel scrolled in window {}: delta=({}, {})",
                    window_id,
                    x,
                    y
                );
            }
            _ => {
                log::trace!("Unhandled event: {:?}", event);
            }
        }
        Ok(true)
    }

    pub async fn main(&mut self) -> Result<()> {
        let mut event_pump = self.sdl.event_pump().map_err(|e| anyhow!(e))?;
        let mut last_frame_time = Instant::now();
        'running: loop {
            // Read messages from the server
            match self.stream.receive().await {
                Ok(event) => {
                    if !self.handle_server_event(event).await? {
                        break 'running;
                    }
                }
                Err(err) => match err.kind() {
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => (), // No data available yet, do nothing
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

            // Events from SDL windows
            for event in event_pump.poll_iter() {
                if !self.handle_window_event(event).await? {
                    break 'running;
                }
            }

            // Sleep to maintain frame rate
            let elapsed_time = last_frame_time.elapsed().as_nanos() as u64;
            if elapsed_time < FRAME_TIME {
                tokio::time::sleep(Duration::from_nanos(FRAME_TIME - elapsed_time)).await;
            } else {
                log::trace!(
                    "Frame time exceeded: {} ns (max: {} ns)",
                    elapsed_time,
                    FRAME_TIME
                );
            }
            last_frame_time = Instant::now();
        }
        for window_id in self.windows.keys().cloned().collect::<Vec<_>>() {
            self.destroy_window(window_id).await?;
        }
        Ok(())
    }

    async fn handle_server_event(&mut self, event: ServerEvent) -> Result<bool> {
        match event {
            ServerEvent::StatusUpdate(status_update) => {
                self.handle_status_update(status_update).await
            }
            ServerEvent::Frame(frame) => self.render_frame(frame),
            other => {
                log::error!("Unexpected server event: {:?}", other);
                return Err(anyhow!("Unexpected server event"));
            }
        }
    }

    async fn handle_status_update(&mut self, status_update: StatusUpdate) -> Result<bool> {
        match status_update.kind.try_into()? {
            StatusType::Exit => {
                log::trace!("Server gracefully disconnected!");
                Ok(false)
            }
            StatusType::Info => {
                let details = status_update.details.ok_or(anyhow!("Missing details"))?;
                let Details::Info(info) = details else {
                    log::warn!("Received unexpected status update message, skipping.");
                    return Ok(true);
                };
                log::info!("Server info: {}", info.message);
                Ok(true)
            }
            StatusType::Warning => {
                let details = status_update.details.ok_or(anyhow!("Missing details"))?;
                let Details::Warning(warning) = details else {
                    log::warn!("Received unexpected status update message, skipping.");
                    return Ok(true);
                };
                log::warn!("Server warning: {}", warning.message);
                Ok(true)
            }
            StatusType::Error => {
                let details = status_update.details.ok_or(anyhow!("Missing details"))?;
                let Details::Error(error) = details else {
                    log::warn!("Received unexpected status update message, skipping.");
                    return Ok(true);
                };
                log::error!("Server error: {}", error.message);
                Ok(true)
            }
        }
    }

    fn render_frame(&mut self, frame: Frame) -> Result<bool> {
        if frame.segments.is_empty() || frame.width == 0 || frame.height == 0 {
            log::warn!("Received empty frame, skipping rendering.");
            return Ok(true); // Keep going
        }
        log::debug!(
            "Received frame of size {}x{} and {} segments",
            frame.width,
            frame.height,
            frame.segments.len()
        );
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
            // Ensure the texture does not blend with the existing canvas contents.
            let _ = texture.set_blend_mode(BlendMode::None);
            // Clear the canvas first so previous frames don't persist beneath the new one.
            win.canvas.set_draw_color(Color::BLACK);
            win.canvas.clear();
            // Apply all segments of the frame to the window
            for segment in &frame.segments {
                if segment.width == 0 || segment.height == 0 {
                    log::warn!("Received empty segment, skipping rendering.");
                    continue;
                }
                let pixel_data = if let Some(compression) = self.compression {
                    match compression {
                        server_hello_ack::Compression::Zstd(_zstd) => {
                            let mut decoder =
                                libgsh::zstd::stream::Decoder::new(&segment.data[..])?;
                            let expected_len =
                                segment.width as usize * segment.height as usize * pixel_bytes;
                            let mut out = Vec::with_capacity(expected_len);
                            decoder.read_to_end(&mut out)?;
                            out
                        }
                    }
                } else {
                    segment.data.clone()
                };
                texture.update(
                    Some(Rect::new(
                        segment.x,
                        segment.y,
                        segment.width,
                        segment.height,
                    )),
                    &pixel_data,
                    segment.width as usize * pixel_bytes,
                )?;
            }
            win.canvas
                .copy(&texture, None, None)
                .map_err(|e| anyhow!(e))?;
            win.canvas.present();
            log::trace!("Updated window ID {}", server_window_id);
        } else {
            log::warn!(
                "Server Window ID {} not found in mapping (not rendered)",
                server_window_id
            );
        }
        Ok(true) // Keep going
    }
}
