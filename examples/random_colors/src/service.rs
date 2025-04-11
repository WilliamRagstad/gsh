use anyhow::Result;
use shared::protocol::{frame_data::FrameFormat, FrameData};
use shared::ClientEvent;

pub struct Service {
    client_sender: std::sync::mpsc::Sender<FrameData>,
    client_receiver: std::sync::mpsc::Receiver<ClientEvent>,
    frame_width: usize,
    frame_height: usize,
}

impl Service {
    pub fn new(
        client_sender: std::sync::mpsc::Sender<FrameData>,
        client_receiver: std::sync::mpsc::Receiver<ClientEvent>,
    ) -> Self {
        Self {
            client_sender,
            client_receiver,
            frame_width: 250,
            frame_height: 250,
        }
    }

    fn new_frame(&self, r: u8, g: u8, b: u8) -> FrameData {
        let format = FrameFormat::Rgba;
        let mut frame = vec![0; self.frame_width * self.frame_height * 4];
        for i in 0..self.frame_width * self.frame_height {
            frame[i * 4] = r;
            frame[i * 4 + 1] = g;
            frame[i * 4 + 2] = b;
            frame[i * 4 + 3] = 255;
        }
        FrameData {
            image_data: frame,
            width: self.frame_width as u32,
            height: self.frame_height as u32,
            format: format as i32,
        }
    }

    fn random_color() -> (u8, u8, u8) {
        let r = rand::random::<u8>();
        let g = rand::random::<u8>();
        let b = rand::random::<u8>();
        (r, g, b)
    }

    pub fn main(self) -> Result<()> {
        log::trace!("Service started...");
        let mut fill = Self::random_color();
        let mut changed = true;
        loop {
            match self.client_receiver.try_recv() {
                Ok(ClientEvent::StatusUpdate(status_update)) => {
                    log::trace!("StatusUpdate: {:?}", status_update);
                    if status_update.kind
                        == shared::protocol::status_update::StatusType::Exit as i32
                    {
                        log::trace!("Received graceful exit status, closing service...");
                        break;
                    }
                }
                Ok(ClientEvent::UserInput(input)) => {
                    log::trace!("Received UserInput: {:?}", input);
                    fill = Self::random_color();
                    changed = true;
                }
                Err(e) => match e {
                    std::sync::mpsc::TryRecvError::Empty => (),
                    std::sync::mpsc::TryRecvError::Disconnected => {
                        log::trace!("Client disconnected, exiting...");
                        break;
                    }
                },
            }
            if changed {
                log::trace!("Sending frame to client...");
                self.client_sender
                    .send(self.new_frame(fill.0, fill.1, fill.2))?;
                changed = false;
            }
        }
        Ok(())
    }
}
