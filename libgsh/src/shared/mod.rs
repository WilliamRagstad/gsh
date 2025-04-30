pub mod r#async;
pub mod sync;

pub use prost;

pub mod protocol {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    StatusUpdate(protocol::StatusUpdate),
    UserInput(protocol::UserInput),
}

pub const PROTOCOL_VERSION: u32 = 1;

type LengthType = u32;
const LENGTH_SIZE: usize = std::mem::size_of::<LengthType>();
