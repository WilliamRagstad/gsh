use super::server::Messages;
use shared::{
    prost::Message,
    protocol::{status_update::StatusType, ServerHelloAck, StatusUpdate, UserInput},
    ClientEvent,
};
use std::io::Write;

#[derive(Debug, thiserror::Error)]
pub enum SerivceError {
    IoError(#[from] std::io::Error),
}

impl std::fmt::Display for SerivceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SerivceError::IoError(err) => write!(f, "IO error: {}", err),
        }
    }
}

pub type Result<T> = std::result::Result<T, SerivceError>;

/// A trait for a simple service that can be run in a separate thread.
/// The service is responsible for handling client events and sending frames to the client.
pub trait SimpleService {
    fn new() -> Self;

    /// Initial window setting preferences for the service.\
    /// This is used in the `handshake_server` function to set the initial window settings for the client.
    /// This is optional and can be overridden by the service implementation.
    /// If not provided, the client may use its own default settings.
    fn server_hello() -> ServerHelloAck;

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
                    std::io::ErrorKind::WouldBlock => (), // No data available yet, do nothing
                    _ => {
                        log::error!("Error reading message: {}", err);
                        allow_wouldblock(self.on_exit(&mut messages))?;
                        break 'running;
                    }
                },
            };

            // Perform periodic tasks in the service
            allow_wouldblock(self.on_tick(&mut messages))?;
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
            SerivceError::IoError(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                log::trace!("Caught WouldBlock error, returning default value.");
                Ok(T::default())
            }
            _ => result,
        },
    }
}
