use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::process_buffer::AudioBufferError;

#[derive(Debug)]
pub enum PluginError {
    InvalidParameter,
    InvalidState,
    UnsupportedHostGuiThreadingModel,
    RequiresInactive,
    Message(&'static str),
}

impl Display for PluginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParameter => f.write_str("invalid parameter"),
            Self::InvalidState => f.write_str("invalid state"),
            Self::UnsupportedHostGuiThreadingModel => {
                f.write_str("unsupported host GUI threading model")
            }
            Self::RequiresInactive => f.write_str("operation requires inactive processing state"),
            Self::Message(message) => f.write_str(message),
        }
    }
}

impl Error for PluginError {}

pub type PluginResult<T> = Result<T, PluginError>;

impl From<AudioBufferError> for PluginError {
    fn from(_value: AudioBufferError) -> Self {
        Self::InvalidState
    }
}
