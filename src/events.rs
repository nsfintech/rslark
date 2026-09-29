//! Event names and raw event payloads.

/// Event callback name supplied by the platform.
pub type EventName = String;

/// An event payload preserved as raw JSON for forward compatibility.
#[cfg(feature = "serde")]
#[derive(Clone, Debug, PartialEq)]
pub struct RawEvent {
    /// Callback name, such as `im.message.receive_v1`.
    pub event_type: EventName,
    /// Original event payload.
    pub payload: serde_json::Value,
}

/// Message event helper types.
pub mod message {
    /// The platform message identifier.
    pub type MessageId = String;
}
