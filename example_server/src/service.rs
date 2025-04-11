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

    fn new_frame(&self, r: u8, g: u8, b: u8) -> FrameData {
        let mut frame = vec![0; self.frame_width * self.frame_height * 4]; // RGBA
        for i in 0..self.frame_width * self.frame_height {
            frame[i * 4] = r; // R
            frame[i * 4 + 1] = g; // G
            frame[i * 4 + 2] = b; // B
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
        let mut fill = (0x00, 0x00, 0x00); // Initial color (black)
        let mut frame_count = 0;
        let mut last_frame_time = std::time::Instant::now();
        const FPS: u32 = 2;
        loop {
            match self.client_receiver.try_recv() {
                Ok(ClientEvent::StatusUpdate(status)) => {
                    println!("Received StatusUpdate: {:?}", status);
                }
                Ok(ClientEvent::UserInput(input)) => {
                    println!("Received UserInput: {:?}", input);
                    // Random color change
                    fill.0 = rand::random::<u8>();
                    fill.1 = rand::random::<u8>();
                    fill.2 = rand::random::<u8>();
                }
                Err(e) => match e {
                    std::sync::mpsc::TryRecvError::Empty => (), // do nothing, just continue
                    std::sync::mpsc::TryRecvError::Disconnected => {
                        println!("Client disconnected, exiting...");
                        break;
                    }
                },
            }
            // Every frame, send a new frame to the client
            println!("Sending frame to client...");
            self.client_sender
                .send(self.new_frame(fill.0, fill.1, fill.2))?;
            frame_count += 1;
            if frame_count % FPS == 0 {
                let elapsed = last_frame_time.elapsed();
                let sleep_duration = std::time::Duration::from_millis(1000 / FPS as u64) - elapsed;
                if sleep_duration > std::time::Duration::ZERO {
                    std::thread::sleep(sleep_duration);
                }
                last_frame_time = std::time::Instant::now();
            }
        }
        Ok(())
    }
}
