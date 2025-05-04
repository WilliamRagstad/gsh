use super::Messages;
use crate::shared::{
    auth::AuthVerifier,
    prost::Message,
    protocol::{status_update::StatusType, ServerHelloAck, StatusUpdate, UserInput},
    ClientEvent,
};
use crate::{Result, ServiceError};
use std::io::Write;

/// A trait for a simple service that can be run in a separate thread.
/// The service is responsible for handling client events and sending frames to the client.
pub trait SimpleService: Clone {
    /// Initial window setting preferences for the service.\
    /// This is used in the `handshake_server` function to set the initial window settings for the client.
    /// This is optional and can be overridden by the service implementation.
    /// If not provided, the client may use its own default settings.
    fn server_hello(&self) -> ServerHelloAck;

    /// Auth verifier for the service.\
    /// This is used to verify the client authentication method.
    fn auth_verifier(&self) -> Option<AuthVerifier> {
        None
    }

    /// Main event loop for the service.\
    /// This is running in a separate thread, handling client events and sending frames back to the client.
    fn main(self, messages: Messages) -> Result<()>
    where
        Self: Sized;
}

/// A trait extension for `SimpleService` that provides additional default functionality:
/// - default `main` event loop implementation, which handles client events and calls `tick` and `handle_event` methods.
/// - `events` method to access the event receiver.
/// - `tick` method to perform periodic tasks.
/// - `handle_event` method to handle client events.
pub trait SimpleServiceExt: SimpleService {
    const MAX_FPS: u32 = 60;
    const FRAME_TIME_NS: u64 = 1_000_000_000 / Self::MAX_FPS as u64; // in nanoseconds
    /// Startup function for the service.\
    /// This is called when the service is started and can be used to perform any necessary initialization.
    fn on_startup(&mut self, _messages: &mut Messages) -> Result<()> {
        Ok(())
    }

    /// Handle periodic tasks in the service.\
    /// This is called each iteration in the default `main` implementation event loop to perform any necessary updates.
    fn on_tick(&mut self, _messages: &mut Messages) -> Result<()> {
        Ok(())
    }

    /// Handle client events in the service.\
    /// This is called for each `ClientEvent` received in the default `main` implementation event loop.
    #[allow(unused_variables)]
    fn on_event(&mut self, messages: &mut Messages, event: ClientEvent) -> Result<()> {
        log::trace!("Got event: {:?}", event);
        Ok(())
    }

    /// Graceful exit of the service.\
    /// This is called when the service receives a `StatusUpdate` event with `Exit` status.
    fn on_exit(&mut self, _messages: &mut Messages) -> Result<()> {
        log::trace!("Exiting service...");
        Ok(())
    }

    /// Main event loop for the service.\
    /// This is running in a separate thread, handling client events and sending frames back to the client.
    fn main(mut self, mut messages: Messages) -> Result<()>
    where
        Self: Sized,
    {
        allow_wouldblock(self.on_startup(&mut messages))?;

        // Set the socket to non-blocking mode
        // All calls to `read_message` will return immediately, even if no data is available
        messages.get_stream().sock.set_nonblocking(true)?;

        log::trace!("Starting service main loop...");
        let mut last_frame_time = std::time::Instant::now();
        'running: loop {
            // Read messages from the client connection
            // This is a non-blocking call, so it will return immediately even if no data is available
            match messages.read_message() {
                Ok(buf) => {
                    if let Ok(status_update) = StatusUpdate::decode(&buf[..]) {
                        if status_update.kind == StatusType::Exit as i32 {
                            log::trace!("Client gracefully disconnected!");
                            messages.get_stream().conn.send_close_notify();
                            let _ = messages.get_stream().flush();
                            let _ = messages
                                .get_stream()
                                .sock
                                .shutdown(std::net::Shutdown::Both);
                            allow_wouldblock(self.on_exit(&mut messages))?;
                            drop(messages);
                            break 'running;
                        }
                        allow_wouldblock(
                            self.on_event(&mut messages, ClientEvent::StatusUpdate(status_update)),
                        )?;
                    } else if let Ok(user_input) = UserInput::decode(&buf[..]) {
                        allow_wouldblock(
                            self.on_event(&mut messages, ClientEvent::UserInput(user_input)),
                        )?;
                    } else {
                        log::trace!("Received data: {:?}", &buf[..]);
                        log::trace!("Unknown message type, ignoring...");
                    }
                }
                Err(err) => match err.kind() {
                    std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::NotConnected => {
                        log::trace!("Client disconnected!");
                        allow_wouldblock(self.on_exit(&mut messages))?;
                        break 'running;
                    }
                    std::io::ErrorKind::WouldBlock => {
                        // No data available yet, do nothing
                    }
                    _ => {
                        log::error!("Error reading message: {}", err);
                        allow_wouldblock(self.on_exit(&mut messages))?;
                        break 'running;
                    }
                },
            };

            // Perform periodic tasks in the service
            allow_wouldblock(self.on_tick(&mut messages))?;

            // Sleep for the tick interval to maintain the desired FPS
            std::thread::sleep(std::time::Duration::from_nanos(Self::FRAME_TIME_NS));

            // Sleep for the tick interval to maintain the desired FPS
            let elapsed_time = last_frame_time.elapsed().as_nanos() as u64;
            if elapsed_time < Self::FRAME_TIME_NS {
                std::thread::sleep(std::time::Duration::from_nanos(
                    Self::FRAME_TIME_NS - elapsed_time,
                ));
            }
            last_frame_time = std::time::Instant::now();
        }
        log::trace!("Service main loop exited.");
        Ok(())
    }
}

/// A helper function to handle `WouldBlock` errors in the service.\
/// If the error is of type `WouldBlock`, it returns a default value instead of an error.\
/// This is useful for non-blocking IO operations where the operation would block if no data is available.
///
/// ## Note
/// This is a bit of a hack, and should be used with caution.
fn allow_wouldblock<T: Default>(result: Result<T>) -> Result<T> {
    match &result {
        Ok(_) => result,
        Err(err) => match err {
            ServiceError::IoError(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                log::trace!("Caught WouldBlock error, returning default value.");
                Ok(T::default())
            }
            _ => result,
        },
    }
}
