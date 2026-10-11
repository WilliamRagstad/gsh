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

/// Per-client service that handles input and sends frames.
#[async_trait]
pub trait GshService: Clone + Send + Sync + 'static {
    /// Initial window settings, frame format, compression, and authentication method.
    fn server_hello(&self) -> ServerHelloAck;

    /// Authentication verifier; `None` disables authentication.
    fn auth_verifier(&self) -> Option<AuthVerifier> {
        None
    }

    /// Runs the per-client event loop.
    async fn main(self, stream: ServerStream) -> Result<()>
    where
        Self: Sized;
}

/// Default event loop with startup, periodic tick, input, and exit hooks.
#[async_trait]
pub trait GshServiceExt: GshService {
    const MAX_FPS: u32 = 60;
    const FRAME_TIME_NS: u64 = 1_000_000_000 / Self::MAX_FPS as u64;
    /// Called once before the event loop starts.
    async fn on_startup(&mut self, _stream: &mut ServerStream) -> Result<()> {
        Ok(())
    }

    /// Called at the interval set by `FRAME_TIME_NS`.
    async fn on_tick(&mut self, _stream: &mut ServerStream) -> Result<()> {
        Ok(())
    }

    /// Handles a client event.
    #[allow(unused_variables)]
    async fn on_event(&mut self, stream: &mut ServerStream, event: ClientEvent) -> Result<()> {
        log::trace!("Got event: {:?}", event);
        Ok(())
    }

    /// Called on client exit or disconnection.
    async fn on_exit(&mut self, _stream: &mut ServerStream) -> Result<()> {
        log::trace!("Exiting service...");
        Ok(())
    }

    /// Runs startup, input/tick dispatch, and exit handling.
    async fn main(mut self, mut stream: ServerStream) -> Result<()>
    where
        Self: Sized,
    {
        self.on_startup(&mut stream).await?;

        log::trace!("Starting service main loop...");
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
                    self.on_tick(&mut stream).await?;
                }
            }
        }
        log::trace!("Service main loop exited.");
        Ok(())
    }
}
