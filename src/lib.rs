//! A Rust SDK skeleton for Feishu and Lark.
//!
//! This crate establishes the public module boundaries for client configuration,
//! HTTP, WebSocket events, messaging, and interactive cards. Network operations
//! are not implemented yet.

pub mod auth;
pub mod card;
pub mod client;
pub mod error;
pub mod events;
pub mod http;
pub mod im;
pub mod ws;

pub use client::{Client, ClientConfig};
pub use error::{Error, Result};
