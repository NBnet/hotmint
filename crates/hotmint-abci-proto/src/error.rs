use std::{error::Error, fmt};

/// An invalid protobuf frame or a semantically invalid ABCI message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The frame could not be decoded as protobuf.
    Protobuf(prost::DecodeError),
    /// The decoded message has a missing or invalid field.
    InvalidMessage(String),
}

impl DecodeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::InvalidMessage(message.into())
    }
}

impl From<prost::DecodeError> for DecodeError {
    fn from(error: prost::DecodeError) -> Self {
        Self::Protobuf(error)
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protobuf(error) => error.fmt(f),
            Self::InvalidMessage(message) => write!(f, "invalid ABCI message: {message}"),
        }
    }
}

impl Error for DecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Protobuf(error) => Some(error),
            Self::InvalidMessage(_) => None,
        }
    }
}
