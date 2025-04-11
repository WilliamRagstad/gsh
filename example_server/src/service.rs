use shared::protocol::{frame_data::FrameFormat, FrameData, StatusUpdate, UserInput};

pub enum ClientEvent {
    StatusUpdate(StatusUpdate),
    UserInput(UserInput),
}

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
            frame_width: 420,
            frame_height: 180,
        }
    }

    fn new_frame(&self, fill: u8) -> FrameData {
        let mut frame = vec![0; self.frame_width * self.frame_height * 4]; // RGBA
        for i in 0..self.frame_width * self.frame_height {
            frame[i * 4] = fill; // R
            frame[i * 4 + 1] = fill; // G
            frame[i * 4 + 2] = fill; // B
            frame[i * 4 + 3] = 255; // A
        }
        FrameData {
            image_data: frame,
            width: self.frame_width as u32,
            height: self.frame_height as u32,
            format: FrameFormat::Rgba as i32,
        }
    }

    pub fn main(self) -> Result<(), Box<dyn std::error::Error>> {
        println!("Service started...");
        loop {
            match self.client_receiver.try_recv() {
                Ok(ClientEvent::StatusUpdate(status)) => {
                    println!("Received StatusUpdate: {:?}", status);
                    self.client_sender.send(self.new_frame(0x0F))?; // Placeholder for actual frame data
                }
                Ok(ClientEvent::UserInput(input)) => {
                    println!("Received UserInput: {:?}", input);
                    self.client_sender.send(self.new_frame(0xFF))?; // Placeholder for actual frame data
                }
                Err(e) => match e {
                    std::sync::mpsc::TryRecvError::Empty => (), // do nothing, just continue
                    std::sync::mpsc::TryRecvError::Disconnected => {
                        println!("Client disconnected, exiting...");
                        break;
                    }
                },
            }
        }
        Ok(())
    }
}
