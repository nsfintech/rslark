//! Error types used by the SDK.

use thiserror::Error;

/// Result type used by SDK operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by SDK operations.
#[derive(Debug, Error)]
pub enum Error {
    /// A required client value is missing or invalid.
    #[error("invalid client configuration: {0}")]
    Config(String),
    /// A transport request failed before a platform response could be parsed.
    #[cfg(feature = "http")]
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// A JSON value could not be serialized or deserialized.
    #[cfg(feature = "serde")]
    #[error("JSON operation failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Feishu or Lark returned a non-zero business code.
    #[error("API error {code}: {msg}")]
    Api {
        /// Platform business error code.
        code: i64,
        /// Platform error message.
        msg: String,
        /// Request identifier returned by the platform, when present.
        request_id: Option<String>,
    },
    /// An OAuth user-authorization request failed.
    #[cfg(feature = "http")]
    #[error("OAuth error {code}: {error}: {error_description}")]
    OAuth {
        /// OAuth or platform error code.
        code: i64,
        /// OAuth error type, such as `access_denied`.
        error: String,
        /// Human-readable OAuth error description.
        error_description: String,
        /// Request identifier returned by the platform, when present.
        request_id: Option<String>,
    },
    /// A WebSocket operation failed.
    #[cfg(feature = "websocket")]
    #[error("WebSocket operation failed: {0}")]
    WebSocket(#[source] Box<tokio_tungstenite::tungstenite::Error>),
    /// The server returned a value that cannot be used safely.
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

#[cfg(feature = "websocket")]
impl From<tokio_tungstenite::tungstenite::Error> for Error {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(error))
    }
}
