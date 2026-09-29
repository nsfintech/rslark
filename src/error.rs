//! Error types used by the SDK.

/// Result type used by SDK operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by SDK operations.
#[derive(Debug)]
pub enum Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, _formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {}
    }
}

impl std::error::Error for Error {}
