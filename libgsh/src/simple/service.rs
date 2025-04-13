use std::sync::mpsc::{Receiver, Sender};

use shared::{
    protocol::{FrameData, WindowSettings},
    ClientEvent,
};

/// A trait for a simple service that can be run in a separate thread.
/// The service is responsible for handling client events and sending frames to the client.
pub trait SimpleService {
    fn new(frames: Sender<FrameData>, events: Receiver<ClientEvent>) -> Self;

    /// Initial window setting preferences for the service.\
    /// This is used in the `handshake_server` function to set the initial window settings for the client.
    /// This is optional and can be overridden by the service implementation.
    /// If not provided, the client may use its own default settings.
    fn initial_window_settings() -> Option<WindowSettings> {
        None
    }

    /// Main event loop for the service.\
    /// This is running in a separate thread, handling client events and sending frames back to the client.
    fn main(self) -> Result<(), Box<dyn std::error::Error>>
    where
        Self: Sized;
}

/// A trait extension for `SimpleService` that provides additional default functionality:
/// - default `main` event loop implementation, which handles client events and calls `tick` and `handle_event` methods.
/// - `events` method to access the event receiver.
/// - `tick` method to perform periodic tasks.
/// - `handle_event` method to handle client events.
pub trait SimpleServiceExt: SimpleService {
    /// Get the event receiver for the service.\
    /// This is used in the default `main` implementation to receive events from the client.
    fn events(&self) -> &Receiver<ClientEvent>;

    /// Handle periodic tasks in the service.\
    /// This is called each iteration in the default `main` implementation event loop to perform any necessary updates.
    fn tick(&mut self) -> Result<(), Box<dyn std::error::Error>>;

    /// Handle client events in the service.\
    /// This is called for each `ClientEvent` received in the default `main` implementation event loop.
    fn handle_event(&mut self, event: ClientEvent) -> Result<(), Box<dyn std::error::Error>>;

    /// Main event loop for the service.\
    /// This is running in a separate thread, handling client events and sending frames back to the client.
    fn main(mut self) -> Result<(), Box<dyn std::error::Error>>
    where
        Self: Sized,
    {
        log::trace!("Starting service main loop...");
        loop {
            match self.events().try_recv() {
                Ok(ClientEvent::StatusUpdate(status_update)) => {
                    if status_update.kind
                        == shared::protocol::status_update::StatusType::Exit as i32
                    {
                        log::trace!("Received graceful exit status, closing service...");
                        break;
                    }
                    self.handle_event(ClientEvent::StatusUpdate(status_update))?;
                }
                Ok(event) => self.handle_event(event)?,
                Err(e) => match e {
                    std::sync::mpsc::TryRecvError::Empty => (),
                    std::sync::mpsc::TryRecvError::Disconnected => {
                        log::trace!("Client disconnected, exiting...");
                        break;
                    }
                },
            }
            self.tick()?;
        }
        log::trace!("Service main loop exited.");
        Ok(())
    }
}
