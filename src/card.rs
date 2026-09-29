//! Card JSON helpers and CardKit API access.

use crate::{Error, Result, client::Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A raw interactive card. Both Card JSON 2.0 and legacy platform card JSON are
/// accepted; validation of card structure is performed by the platform.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Card(Value);

impl Card {
    /// Create a card from an existing JSON value.
    pub fn new(value: Value) -> Result<Self> {
        if !value.is_object() {
            return Err(Error::Config("card JSON must be an object".into()));
        }
        Ok(Self(value))
    }

    /// Return the underlying card JSON.
    pub fn as_json(&self) -> &Value {
        &self.0
    }

    /// Convert the card to the string content used by IM message APIs.
    pub fn as_message_content(&self) -> Result<String> {
        Ok(self.0.to_string())
    }
}

/// Build a Card JSON 2.0 object with common card elements.
#[derive(Debug, Default)]
pub struct CardBuilder {
    title: Option<String>,
    template: Option<String>,
    elements: Vec<Value>,
}

impl CardBuilder {
    /// Create an empty Card JSON 2.0 builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the card header title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set a CardKit template header color.
    pub fn template(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }

    /// Append a Markdown element.
    pub fn markdown(mut self, content: impl Into<String>) -> Self {
        self.elements.push(serde_json::json!({
            "tag": "markdown",
            "content": content.into(),
        }));
        self
    }

    /// Append an arbitrary Card JSON 2.0 element.
    pub fn element(mut self, element: Value) -> Self {
        self.elements.push(element);
        self
    }

    /// Build the final card.
    pub fn build(self) -> Result<Card> {
        let mut card = serde_json::json!({
            "schema": "2.0",
            "body": { "elements": self.elements },
        });
        if let Some(title) = self.title {
            let mut header = serde_json::json!({
                "title": { "tag": "plain_text", "content": title }
            });
            if let Some(template) = self.template {
                header["template"] = Value::String(template);
            }
            card["header"] = header;
        }
        Card::new(card)
    }
}

/// Request body for creating a CardKit card entity.
#[derive(Debug, Serialize)]
pub struct CreateCardRequest {
    #[serde(rename = "type")]
    pub card_type: String,
    pub data: String,
}

/// Response returned after creating a CardKit card entity.
#[derive(Clone, Debug, Deserialize)]
pub struct CreatedCard {
    pub card_id: String,
}

impl Client {
    /// Create a reusable CardKit entity from Card JSON.
    pub async fn create_card(&self, card: &Card) -> Result<CreatedCard> {
        let request = CreateCardRequest {
            card_type: "card_json".into(),
            data: card.as_message_content()?,
        };
        self.request_json(
            reqwest::Method::POST,
            "/open-apis/cardkit/v1/cards",
            None,
            Some(&request),
        )
        .await
    }

    /// Send a card entity created by CardKit.
    pub async fn send_card_entity(
        &self,
        receive_id_type: crate::im::ReceiveIdType,
        receive_id: impl Into<String>,
        card_id: impl Into<String>,
        uuid: Option<String>,
    ) -> Result<crate::im::Message> {
        let content = serde_json::json!({
            "type": "card",
            "data": { "card_id": card_id.into() }
        })
        .to_string();
        self.send_message(
            receive_id_type,
            crate::im::SendMessageRequest {
                receive_id: receive_id.into(),
                msg_type: "interactive".into(),
                content,
                uuid,
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_card_json_and_cardkit_payload() {
        let card = CardBuilder::new()
            .title("Deployment")
            .template("blue")
            .markdown("Deployment succeeded")
            .build()
            .unwrap();
        let json = card.as_json();

        assert_eq!(json["schema"], "2.0");
        assert_eq!(json["header"]["title"]["content"], "Deployment");
        assert_eq!(json["body"]["elements"][0]["tag"], "markdown");
        assert_eq!(
            card.as_message_content().unwrap(),
            serde_json::to_string(json).unwrap()
        );
    }
}
