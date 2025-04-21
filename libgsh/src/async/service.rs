use super::server::AsyncMessages;
use crate::Result;
use async_trait::async_trait;
use shared::{
    prost::Message,
    protocol::{status_update::StatusType, ServerHelloAck, StatusUpdate, UserInput},
    ClientEvent,
};
use tokio::io::AsyncWriteExt;

/// A trait for an async service that can be run in a separate thread.
/// The service is responsible for handling client events and sending frames to the client.
#[async_trait]
pub trait AsyncService: Send + Sync + 'static {
    /// Creates a new instance of the service.\
    fn new() -> Self;

    /// Initial window setting preferences for the service.\
    /// This is used in the `handshake_server` function to set the initial window settings for the client.
    /// This is optional and can be overridden by the service implementation.
    /// If not provided, the client may use its own default settings.
    fn server_hello() -> ServerHelloAck;

    /// Main event loop for the service.\
    /// This is running in a separate thread, handling client events and sending frames back to the client.
    async fn main<'a>(self, messages: AsyncMessages) -> Result<()>
    where
        Self: Sized + Send + Sync;
}

/// A trait extension for `AsyncService` that provides additional default functionality:
/// - default `main` event loop implementation, which handles client events and calls `tick` and `handle_event` methods.
/// - `events` method to access the event receiver.
/// - `tick` method to perform periodic tasks.
/// - `handle_event` method to handle client events.
#[async_trait]
pub trait AsyncServiceExt: AsyncService {
    const FPS: u32 = 60;
    const TICK_INTERVAL: u64 = 1_000_000_000 / Self::FPS as u64; // in nanoseconds
    /// Startup function for the service.\
    /// This is called when the service is started and can be used to perform any necessary initialization.
    async fn on_startup(&mut self, _messages: &mut AsyncMessages) -> Result<()> {
        Ok(())
    }

    /// Handle periodic tasks in the service.\
    /// This is called each iteration in the default `main` implementation event loop to perform any necessary updates.
    async fn on_tick(&mut self, _messages: &mut AsyncMessages) -> Result<()> {
        Ok(())
    }

    /// Handle client events in the service.\
    /// This is called for each `ClientEvent` received in the default `main` implementation event loop.
    #[allow(unused_variables)]
    async fn on_event(&mut self, messages: &mut AsyncMessages, event: ClientEvent) -> Result<()> {
        log::trace!("Got event: {:?}", event);
        Ok(())
    }

    /// Graceful exit of the service.\
    /// This is called when the service receives a `StatusUpdate` event with `Exit` status.
    async fn on_exit(&mut self, _messages: &mut AsyncMessages) -> Result<()> {
        log::trace!("Exiting service...");
        Ok(())
    }

    /// Main event loop for the service.\
    /// This is running in a separate thread, handling client events and sending frames back to the client.
    async fn main(mut self, mut messages: AsyncMessages) -> Result<()>
    where
        Self: Sized + Send + Sync,
    {
        self.on_startup(&mut messages).await?;

        // Set the socket to non-blocking mode
        // All calls to `read_message` will return immediately, even if no data is available
        // messages.get_stream().sock.set_nonblocking(true)?;

        log::trace!("Starting service main loop...");
        'running: loop {
            // Read messages from the client connection
            // This is a non-blocking call, so it will return immediately even if no data is available
            match messages.read_message().await {
                Ok(buf) => {
                    if let Ok(status_update) = StatusUpdate::decode(&buf[..]) {
                        if status_update.kind == StatusType::Exit as i32 {
                            log::trace!("Client gracefully disconnected!");
                            messages.get_stream().get_mut().1.send_close_notify();
                            let _ = messages.get_stream().get_mut().0.flush().await;
                            let _ = messages.get_stream().get_mut().0.shutdown().await;
                            self.on_exit(&mut messages).await?;
                            drop(messages);
                            break 'running;
                        }
                        self.on_event(&mut messages, ClientEvent::StatusUpdate(status_update))
                            .await?;
                    } else if let Ok(user_input) = UserInput::decode(&buf[..]) {
                        self.on_event(&mut messages, ClientEvent::UserInput(user_input))
                            .await?;
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
                        self.on_exit(&mut messages).await?;
                        break 'running;
                    }
                    std::io::ErrorKind::WouldBlock => {
                        // No data available yet, do nothing
                    }
                    _ => {
                        log::error!("Error reading message: {}", err);
                        self.on_exit(&mut messages).await?;
                        break 'running;
                    }
                },
            };

            // Perform periodic tasks in the service
            self.on_tick(&mut messages).await?;

            // Sleep for the tick interval to maintain the desired FPS
            std::thread::sleep(std::time::Duration::from_nanos(Self::TICK_INTERVAL));
        }
        log::trace!("Service main loop exited.");
        Ok(())
    }
}

// /// A helper function to handle `WouldBlock` errors in the service.\
// /// If the error is of type `WouldBlock`, it returns a default value instead of an error.\
// /// This is useful for non-blocking IO operations where the operation would block if no data is available.
// ///
// /// ## Note
// /// This is a bit of a hack, and should be used with caution.
// fn allow_wouldblock<T: Default>(result: Result<T>) -> Result<T> {
//     match &result {
//         Ok(_) => result,
//         Err(err) => match err {
//             SerivceError::IoError(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
//                 log::trace!("Caught WouldBlock error, returning default value.");
//                 Ok(T::default())
//             }
//             _ => result,
//         },
//     }
// }
