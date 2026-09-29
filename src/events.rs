//! Event payloads and simple event-type routing.

#[cfg(feature = "http")]
use crate::Error;
use crate::Result;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Event callback name supplied by the platform.
pub type EventName = String;

/// A synchronous event handler.
pub type EventHandler =
    Arc<dyn Fn(&RawEvent) -> Result<Option<serde_json::Value>> + Send + Sync + 'static>;

/// An event payload preserved as raw JSON for forward compatibility.
#[derive(Clone, Debug, PartialEq)]
pub struct RawEvent {
    /// Callback name, such as `im.message.receive_v1`.
    pub event_type: EventName,
    /// Original event payload.
    pub payload: Value,
}

impl RawEvent {
    /// Extract the event type from a schema 1.0 or 2.0 platform payload.
    pub fn from_payload(payload: Value) -> Self {
        let event_type = payload
            .get("header")
            .and_then(|header| header.get("event_type"))
            .or_else(|| payload.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Self {
            event_type,
            payload,
        }
    }
}

/// Routes raw events by exact platform event type.
#[derive(Clone, Default)]
pub struct EventRouter {
    handlers: HashMap<EventName, Vec<EventHandler>>,
    fallback: Option<EventHandler>,
}

impl EventRouter {
    /// Create an empty router.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler for an event type such as `im.message.receive_v1`.
    pub fn on<F>(mut self, event_type: impl Into<String>, handler: F) -> Self
    where
        F: Fn(&RawEvent) -> Result<Option<serde_json::Value>> + Send + Sync + 'static,
    {
        self.handlers
            .entry(event_type.into())
            .or_default()
            .push(Arc::new(handler));
        self
    }

    /// Register a handler invoked for event types without an explicit handler.
    pub fn fallback<F>(mut self, handler: F) -> Self
    where
        F: Fn(&RawEvent) -> Result<Option<serde_json::Value>> + Send + Sync + 'static,
    {
        self.fallback = Some(Arc::new(handler));
        self
    }

    /// Invoke all matching handlers, or the fallback when none match.
    pub fn dispatch(&self, event: &RawEvent) -> Result<Option<serde_json::Value>> {
        let matching = self.handlers.get(&event.event_type);
        let mut failure = None;
        let mut reply = None;
        if let Some(handlers) = matching {
            for handler in handlers {
                match handler(event) {
                    Ok(value) => {
                        if value.is_some() {
                            reply = value;
                        }
                    }
                    Err(error) => failure = Some(error),
                }
            }
        } else if let Some(handler) = &self.fallback {
            match handler(event) {
                Ok(value) => reply = value,
                Err(error) => failure = Some(error),
            }
        }

        failure.map_or(Ok(reply), Err)
    }
}

/// Common fields from a schema 2.0 event header.
#[derive(Clone, Debug, Deserialize)]
pub struct EventHeader {
    pub event_id: String,
    pub event_type: String,
    #[serde(default)]
    pub create_time: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub tenant_key: Option<String>,
}

/// A typed message-received event.
#[cfg(feature = "http")]
#[derive(Clone, Debug, Deserialize)]
pub struct MessageEvent {
    pub message: crate::im::Message,
    #[serde(default)]
    pub sender: Option<Value>,
}

/// A schema 2.0 card action callback.
#[derive(Clone, Debug, Deserialize)]
pub struct CardActionEvent {
    #[serde(default)]
    pub operator: Option<Value>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub action: Option<Value>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub delivery_type: Option<String>,
    #[serde(default)]
    pub context: Option<Value>,
}

#[cfg(feature = "http")]
impl MessageEvent {
    /// Parse a schema 2.0 `im.message.receive_v1` payload.
    pub fn parse(event: &RawEvent) -> Result<Self> {
        if event.event_type != "im.message.receive_v1" {
            return Err(Error::InvalidResponse(format!(
                "expected im.message.receive_v1, got {}",
                event.event_type
            )));
        }
        let event_body = event
            .payload
            .get("event")
            .cloned()
            .ok_or_else(|| Error::InvalidResponse("message event data is missing".into()))?;
        serde_json::from_value(event_body).map_err(Into::into)
    }
}

#[cfg(feature = "http")]
impl CardActionEvent {
    /// Parse a schema 2.0 `card.action.trigger` payload.
    pub fn parse(event: &RawEvent) -> Result<Self> {
        if event.event_type != "card.action.trigger" {
            return Err(Error::InvalidResponse(format!(
                "expected card.action.trigger, got {}",
                event.event_type
            )));
        }
        let event_body = event
            .payload
            .get("event")
            .cloned()
            .ok_or_else(|| Error::InvalidResponse("card action data is missing".into()))?;
        serde_json::from_value(event_body).map_err(Into::into)
    }
}

#[cfg(all(test, feature = "http"))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn routes_events_by_type_and_uses_fallback_otherwise() {
        let matched = Arc::new(AtomicBool::new(false));
        let fallback = Arc::new(AtomicBool::new(false));
        let matched_handler = Arc::clone(&matched);
        let fallback_handler = Arc::clone(&fallback);
        let router = EventRouter::new()
            .on("test.event", move |_| {
                matched_handler.store(true, Ordering::SeqCst);
                Ok(None)
            })
            .fallback(move |_| {
                fallback_handler.store(true, Ordering::SeqCst);
                Ok(None)
            });

        router
            .dispatch(&RawEvent::from_payload(serde_json::json!({
                "header": { "event_type": "test.event" }
            })))
            .unwrap();
        router
            .dispatch(&RawEvent::from_payload(serde_json::json!({
                "header": { "event_type": "other.event" }
            })))
            .unwrap();

        assert!(matched.load(Ordering::SeqCst));
        assert!(fallback.load(Ordering::SeqCst));
    }

    #[test]
    fn parses_card_action_event() {
        let event = RawEvent::from_payload(serde_json::json!({
            "header": { "event_type": "card.action.trigger" },
            "event": {
                "operator": { "open_id": "ou_1" },
                "action": { "tag": "button", "value": { "action": "approve" } },
                "context": { "open_message_id": "om_1" }
            }
        }));
        let parsed = CardActionEvent::parse(&event).unwrap();

        assert_eq!(parsed.operator.as_ref().unwrap()["open_id"], "ou_1");
        assert_eq!(
            parsed.action.as_ref().unwrap()["value"]["action"],
            "approve"
        );
    }

    #[test]
    fn parses_message_receive_event() {
        let event = RawEvent::from_payload(serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "message": { "message_id": "om_1", "chat_id": "oc_1" },
                "sender": { "sender_id": { "open_id": "ou_1" } }
            }
        }));
        let parsed = MessageEvent::parse(&event).unwrap();

        assert_eq!(parsed.message.message_id.as_deref(), Some("om_1"));
        assert_eq!(parsed.message.chat_id.as_deref(), Some("oc_1"));
    }
}
