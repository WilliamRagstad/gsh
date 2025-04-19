use super::server::Messages;
use shared::{
    prost::Message,
    protocol::{status_update::StatusType, ServerHelloAck, StatusUpdate, UserInput},
    ClientEvent,
};
use std::io::Write;

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
    fn main(self, messages: Messages) -> Result<(), Box<dyn std::error::Error>>
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
    fn on_startup(&mut self, _messages: &mut Messages) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Handle periodic tasks in the service.\
    /// This is called each iteration in the default `main` implementation event loop to perform any necessary updates.
    fn on_tick(&mut self, _messages: &mut Messages) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Handle client events in the service.\
    /// This is called for each `ClientEvent` received in the default `main` implementation event loop.
    #[allow(unused_variables)]
    fn on_event(
        &mut self,
        messages: &mut Messages,
        event: ClientEvent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        log::trace!("Got event: {:?}", event);
        Ok(())
    }

    /// Graceful exit of the service.\
    /// This is called when the service receives a `StatusUpdate` event with `Exit` status.
    fn on_exit(&mut self, _messages: &mut Messages) -> Result<(), Box<dyn std::error::Error>> {
        log::trace!("Exiting service...");
        Ok(())
    }

    /// Main event loop for the service.\
    /// This is running in a separate thread, handling client events and sending frames back to the client.
    fn main(mut self, mut messages: Messages) -> Result<(), Box<dyn std::error::Error>>
    where
        Self: Sized,
    {
        self.on_startup(&mut messages)?;

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
                            self.on_exit(&mut messages)?;
                            drop(messages);
                            break 'running;
                        }
                        self.on_event(&mut messages, ClientEvent::StatusUpdate(status_update))?;
                    } else if let Ok(user_input) = UserInput::decode(&buf[..]) {
                        self.on_event(&mut messages, ClientEvent::UserInput(user_input))?;
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
                        self.on_exit(&mut messages)?;
                        break 'running;
                    }
                    std::io::ErrorKind::WouldBlock => (), // No data available yet, do nothing
                    _ => {
                        log::error!("Error reading message: {}", err);
                        self.on_exit(&mut messages)?;
                        break 'running;
                    }
                },
            };

            // Perform periodic tasks in the service
            self.on_tick(&mut messages)?;
        }
        log::trace!("Service main loop exited.");
        Ok(())
    }
}
