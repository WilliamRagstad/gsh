use std::sync::mpsc::{Receiver, Sender};

use shared::{
    protocol::{FrameData, WindowSettings},
    ClientEvent,
};

/// A trait for a simple service that can be run in a separate thread.
/// The service is responsible for handling client events and sending frames to the client.
pub trait SimpleService {
    fn new(frames: Sender<FrameData>, events: Receiver<ClientEvent>) -> Self;
    fn initial_window_settings() -> Option<WindowSettings> {
        None
    }
    fn main(self) -> Result<(), Box<dyn std::error::Error>>;
}

/// A trait extension for `SimpleService` that provides additional default functionality:
/// - default `main` event loop implementation, which handles client events and calls `tick` and `handle_event` methods.
/// - `events` method to access the event receiver.
/// - `tick` method to perform periodic tasks.
/// - `handle_event` method to handle client events.
pub trait SimpleServiceExt: SimpleService {
    fn events(&self) -> &Receiver<ClientEvent>;
    fn tick(&mut self) -> Result<(), Box<dyn std::error::Error>>;
    fn handle_event(&mut self, event: ClientEvent) -> Result<(), Box<dyn std::error::Error>>;
    fn main(mut self) -> Result<(), Box<dyn std::error::Error>>
    where
        Self: Sized,
    {
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
        Ok(())
    }
}
