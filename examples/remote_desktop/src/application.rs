use crate::frame_source::{FrameHub, FrameSource};
use libgsh::shared::protocol::UserInput;

/// Transport-independent remote desktop application state.
#[derive(Clone)]
pub struct RemoteDesktopApplication {
    frames: FrameHub,
}

impl RemoteDesktopApplication {
    pub fn new(frames: FrameHub) -> Self {
        Self { frames }
    }

    pub fn subscribe_frames(&self) -> FrameSource {
        self.frames.subscribe()
    }

    pub fn handle_input(&self, input: &UserInput) {
        log::info!("Received application input: {input:?}");
    }
}
