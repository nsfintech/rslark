//! A Rust SDK for the Feishu and Lark Open APIs.

pub mod auth;
#[cfg(feature = "http")]
pub mod card;
pub mod client;
pub mod error;
#[cfg(feature = "serde")]
pub mod events;
pub mod http;
#[cfg(feature = "http")]
pub mod im;
#[cfg(feature = "websocket")]
pub mod ws;

pub use auth::AppCredentials;
#[cfg(feature = "http")]
pub use card::{
    Card, CardBuilder, CardKitCard, CreateCardRequest, CreatedCard, UpdateCardContentRequest,
    UpdateCardRequest, UpdateCardSettingsRequest,
};
pub use client::{Client, ClientConfig};
pub use error::{Error, Result};
#[cfg(feature = "serde")]
pub use events::{EventRouter, RawEvent};
#[cfg(feature = "http")]
pub use im::ReceiveIdType;
#[cfg(feature = "websocket")]
pub use ws::WsClient;
