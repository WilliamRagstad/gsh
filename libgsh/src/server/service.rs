use super::ServerStream;
use crate::{
    shared::{
        auth::AuthVerifier,
        protocol::{client_message::ClientEvent, status_update::StatusType, ServerHelloAck},
    },
    Result,
};
use async_trait::async_trait;
use std::io::ErrorKind;
use tokio::io::AsyncWriteExt;

/// A trait for an async service that can be run in a separate thread.
/// The service is responsible for handling client events and sending frames to the client.
#[async_trait]
pub trait GshService: Clone + Send + Sync + 'static {
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
    async fn main(self, stream: ServerStream) -> Result<()>
    where
        Self: Sized;
}

/// A trait extension for `AsyncService` that provides additional default functionality:
/// - default `main` event loop implementation, which handles client events and calls `tick` and `handle_event` methods.
/// - `events` method to access the event receiver.
/// - `tick` method to perform periodic tasks.
/// - `handle_event` method to handle client events.
#[async_trait]
pub trait GshServiceExt: GshService {
    const MAX_FPS: u32 = 60;
    const FRAME_TIME_NS: u64 = 1_000_000_000 / Self::MAX_FPS as u64; // in nanoseconds
    /// Start up function for the service.\
    /// This is called when the service is started and can be used to perform any necessary initialization.
    async fn on_startup(&mut self, _stream: &mut ServerStream) -> Result<()> {
        Ok(())
    }

    /// Handle periodic tasks in the service.\
    /// This is called each iteration in the default `main` implementation event loop to perform any necessary updates.
    async fn on_tick(&mut self, _stream: &mut ServerStream) -> Result<()> {
        Ok(())
    }

    /// Handle client events in the service.\
    /// This is called for each `ClientEvent` received in the default `main` implementation event loop.
    #[allow(unused_variables)]
    async fn on_event(&mut self, stream: &mut ServerStream, event: ClientEvent) -> Result<()> {
        log::trace!("Got event: {:?}", event);
        Ok(())
    }

    /// Graceful exit of the service.\
    /// This is called when the service receives a `StatusUpdate` event with `Exit` status.
    async fn on_exit(&mut self, _stream: &mut ServerStream) -> Result<()> {
        log::trace!("Exiting service...");
        Ok(())
    }

    /// Main event loop for the service.\
    /// This is running in a separate thread, handling client events and sending frames back to the client.
    async fn main(mut self, mut stream: ServerStream) -> Result<()>
    where
        Self: Sized,
    {
        self.on_startup(&mut stream).await?;

        log::trace!("Starting service main loop...");
        // Use a tokio interval for precise pacing and natural yielding.
        let mut tick = tokio::time::interval(std::time::Duration::from_nanos(Self::FRAME_TIME_NS));
        'running: loop {
            tokio::select! {
                res = stream.receive() => {
                    match res {
                        Ok(ClientEvent::StatusUpdate(status_update)) => {
                            if status_update.kind == StatusType::Exit as i32 {
                                log::trace!("Client gracefully disconnected!");
                                stream.get_inner().get_mut().1.send_close_notify();
                                let _ = stream.get_inner().get_mut().0.flush().await;
                                let _ = stream.get_inner().get_mut().0.shutdown().await;
                                self.on_exit(&mut stream).await?;
                                drop(stream);
                                break 'running;
                            }
                            self.on_event(&mut stream, ClientEvent::StatusUpdate(status_update)).await?;
                        }
                        Ok(ClientEvent::UserInput(user_input)) => {
                            self.on_event(&mut stream, ClientEvent::UserInput(user_input)).await?;
                        }
                        Ok(other) => {
                            log::trace!("Received data: {:?}", &other);
                            log::trace!("Unknown message type, ignoring...");
                        }
                        Err(err) => match err.kind() {
                            ErrorKind::UnexpectedEof
                            | ErrorKind::ConnectionAborted
                            | ErrorKind::ConnectionRefused
                            | ErrorKind::ConnectionReset
                            | ErrorKind::NotConnected => {
                                log::trace!("Client disconnected!");
                                self.on_exit(&mut stream).await?;
                                break 'running;
                            }
                            ErrorKind::WouldBlock | ErrorKind::TimedOut => {
                                // No data available yet, do nothing
                            }
                            _ => {
                                log::error!("Error reading message: {}", err);
                                self.on_exit(&mut stream).await?;
                                break 'running;
                            }
                        },
                    }
                }
                _ = tick.tick() => {
                    // Periodic tick; call on_tick which may render and send frames.
                    self.on_tick(&mut stream).await?;
                }
            }
        }
        log::trace!("Service main loop exited.");
        Ok(())
    }
}
