pub mod convert;
mod error;

pub use error::DecodeError;

/// Generated protobuf types for the Hotmint ABCI protocol.
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/hotmint.abci.rs"));
}
