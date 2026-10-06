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
    streaming_mode: Option<bool>,
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

    /// Enable or disable CardKit streaming mode in the built card config.
    pub fn streaming_mode(mut self, enabled: bool) -> Self {
        self.streaming_mode = Some(enabled);
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

    /// Append a Markdown element with an explicit CardKit element ID.
    ///
    /// The element ID is required for the streaming content-update API.
    pub fn markdown_with_id(
        mut self,
        element_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.elements.push(serde_json::json!({
            "tag": "markdown",
            "element_id": element_id.into(),
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
        if let Some(streaming_mode) = self.streaming_mode {
            card["config"]["streaming_mode"] = Value::Bool(streaming_mode);
        }
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

/// Serialized Card JSON used by CardKit update requests.
#[derive(Clone, Debug, Serialize)]
pub struct CardKitCard {
    /// CardKit payload type; always `card_json` for requests built by the SDK.
    #[serde(rename = "type")]
    pub card_type: String,
    /// Serialized Card JSON 2.0 document.
    pub data: String,
}

impl CardKitCard {
    /// Wrap a Card JSON 2.0 value for a CardKit update request.
    pub fn new(card: &Card) -> Result<Self> {
        ensure_card_json_2_0(card.as_json())?;
        Ok(Self {
            card_type: "card_json".into(),
            data: card.as_message_content()?,
        })
    }
}

/// Request body for creating a CardKit card entity.
#[derive(Debug, Serialize)]
pub struct CreateCardRequest {
    /// CardKit payload type; use `card_json` for Card JSON input.
    #[serde(rename = "type")]
    pub card_type: String,
    /// Serialized Card JSON 2.0 document.
    pub data: String,
}

/// Response returned after creating a CardKit card entity.
#[derive(Clone, Debug, Deserialize)]
pub struct CreatedCard {
    /// Identifier of the created reusable card entity.
    pub card_id: String,
}

/// Request body for streaming a Markdown or plain-text element.
#[derive(Clone, Debug, Serialize)]
pub struct UpdateCardContentRequest {
    /// New full content of the element. CardKit renders the difference as a
    /// typewriter-style update when appropriate.
    pub content: String,
    /// Operation sequence. It must increase strictly for the same card.
    pub sequence: i32,
    /// Optional idempotency UUID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

impl UpdateCardContentRequest {
    /// Create a streaming content update request.
    pub fn new(content: impl Into<String>, sequence: i32) -> Self {
        Self {
            content: content.into(),
            sequence,
            uuid: None,
        }
    }

    /// Set an optional idempotency UUID.
    pub fn with_uuid(mut self, uuid: impl Into<String>) -> Self {
        self.uuid = Some(uuid.into());
        self
    }
}

/// Request body for replacing an entire CardKit card.
#[derive(Clone, Debug, Serialize)]
pub struct UpdateCardRequest {
    /// Complete replacement Card JSON 2.0 value.
    pub card: CardKitCard,
    /// Operation sequence. It must increase strictly for the same card.
    pub sequence: i32,
    /// Optional idempotency UUID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

impl UpdateCardRequest {
    /// Create a full-card replacement request.
    pub fn new(card: &Card, sequence: i32) -> Result<Self> {
        Ok(Self {
            card: CardKitCard::new(card)?,
            sequence,
            uuid: None,
        })
    }

    /// Set an optional idempotency UUID.
    pub fn with_uuid(mut self, uuid: impl Into<String>) -> Self {
        self.uuid = Some(uuid.into());
        self
    }
}

/// Request body for updating CardKit card settings.
#[derive(Clone, Debug, Serialize)]
pub struct UpdateCardSettingsRequest {
    /// Serialized JSON containing supported `config` and `card_link` fields.
    pub settings: String,
    /// Operation sequence. It must increase strictly for the same card.
    pub sequence: i32,
    /// Optional idempotency UUID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

impl UpdateCardSettingsRequest {
    /// Create a settings update request from a JSON object.
    pub fn new(settings: &Value, sequence: i32) -> Result<Self> {
        if !settings.is_object() {
            return Err(Error::Config("card settings JSON must be an object".into()));
        }
        Ok(Self {
            settings: serde_json::to_string(settings)?,
            sequence,
            uuid: None,
        })
    }

    /// Set an optional idempotency UUID.
    pub fn with_uuid(mut self, uuid: impl Into<String>) -> Self {
        self.uuid = Some(uuid.into());
        self
    }
}

impl Client {
    /// Create a reusable CardKit entity from Card JSON 2.0.
    pub async fn create_card(&self, card: &Card) -> Result<CreatedCard> {
        ensure_card_json_2_0(card.as_json())?;
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

    /// Create a CardKit entity with streaming mode enabled.
    ///
    /// The input must be Card JSON 2.0. This method preserves the supplied card
    /// and sets `config.streaming_mode` to `true`; callers choose the initial
    /// elements and element IDs.
    pub async fn create_streaming_card(&self, card: &Card) -> Result<CreatedCard> {
        ensure_card_json_2_0(card.as_json())?;
        let mut streaming_card = card.as_json().clone();
        if !streaming_card.get("config").is_some_and(Value::is_object) {
            streaming_card["config"] = serde_json::json!({});
        }
        let config = streaming_card
            .get_mut("config")
            .ok_or_else(|| Error::Config("card config must be an object".into()))?;
        if !config.is_object() {
            return Err(Error::Config("card config must be an object".into()));
        }
        config["streaming_mode"] = Value::Bool(true);

        self.create_card(&Card::new(streaming_card)?).await
    }

    /// Stream the full new content of a Markdown or plain-text element.
    ///
    /// The card must have been created with streaming mode enabled. `sequence`
    /// must increase strictly across all updates to the same card. The platform
    /// success response contains no business fields, so it is represented by
    /// `()`.
    pub async fn update_card_element_content(
        &self,
        card_id: impl AsRef<str>,
        element_id: impl AsRef<str>,
        request: UpdateCardContentRequest,
    ) -> Result<()> {
        validate_sequence(request.sequence)?;
        self.request_empty(
            reqwest::Method::PUT,
            &format!(
                "/open-apis/cardkit/v1/cards/{}/elements/{}/content",
                card_id.as_ref(),
                element_id.as_ref()
            ),
            None,
            Some(&request),
        )
        .await
    }

    /// Replace an entire CardKit card with new Card JSON 2.0 content.
    ///
    /// `sequence` must increase strictly across all updates to the same card.
    /// The platform success response contains no business fields, so it is
    /// represented by `()`.
    pub async fn update_card(
        &self,
        card_id: impl AsRef<str>,
        request: UpdateCardRequest,
    ) -> Result<()> {
        validate_sequence(request.sequence)?;
        self.request_empty(
            reqwest::Method::PUT,
            &format!("/open-apis/cardkit/v1/cards/{}", card_id.as_ref()),
            None,
            Some(&request),
        )
        .await
    }

    /// Update CardKit settings such as `config.streaming_mode`.
    ///
    /// `sequence` must increase strictly across all updates to the same card.
    /// The platform success response contains no business fields, so it is
    /// represented by `()`.
    pub async fn update_card_settings(
        &self,
        card_id: impl AsRef<str>,
        request: UpdateCardSettingsRequest,
    ) -> Result<()> {
        validate_sequence(request.sequence)?;
        self.request_empty(
            reqwest::Method::PATCH,
            &format!("/open-apis/cardkit/v1/cards/{}/settings", card_id.as_ref()),
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

fn ensure_card_json_2_0(card: &Value) -> Result<()> {
    if card.get("schema").and_then(Value::as_str) != Some("2.0") {
        return Err(Error::Config(
            "CardKit requests require Card JSON schema 2.0".into(),
        ));
    }
    Ok(())
}

fn validate_sequence(sequence: i32) -> Result<()> {
    if sequence <= 0 {
        return Err(Error::Config(
            "CardKit sequence must be a positive int32 value".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "http")]
    mod http {
        use super::*;
        use crate::{Client, auth::AppCredentials, client::ClientConfig};
        use std::{
            io::{Read, Write},
            net::TcpListener,
            sync::mpsc,
            thread,
        };

        #[tokio::test]
        async fn creates_streaming_card_and_maps_api_errors() {
            let (base_url, requests) = mock_http_server(vec![
                (
                    200,
                    r#"{"code":0,"msg":"ok","tenant_access_token":"token","expire":7200}"#,
                ),
                (200, r#"{"code":0,"msg":"ok","data":{"card_id":"card-1"}}"#),
                (400, r#"{"code":131400,"msg":"invalid card"}"#),
            ]);
            let client = Client::with_credentials_and_config(
                AppCredentials::new("app-id", "app-secret"),
                ClientConfig::with_api_base_url(base_url),
            )
            .unwrap();
            let card = CardBuilder::new()
                .markdown_with_id("research_stream", "Researching")
                .build()
                .unwrap();

            let created = client.create_streaming_card(&card).await.unwrap();
            let error = client.create_streaming_card(&card).await.unwrap_err();

            assert_eq!(created.card_id, "card-1");
            assert_api_error(error, 131400, "invalid card");
            let _ = requests.recv().unwrap();
            for request in [&requests.recv().unwrap(), &requests.recv().unwrap()] {
                let (head, body) = split_request(request);
                assert!(head.starts_with("POST /open-apis/cardkit/v1/cards HTTP/1.1"));
                assert!(head.contains("Bearer token"));
                let body: Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["type"], "card_json");
                let data: Value = serde_json::from_str(body["data"].as_str().unwrap()).unwrap();
                assert_eq!(data["schema"], "2.0");
                assert_eq!(data["config"]["streaming_mode"], true);
                assert_eq!(data["body"]["elements"][0]["element_id"], "research_stream");
            }
        }

        #[tokio::test]
        async fn updates_card_element_content_and_maps_api_errors() {
            let (base_url, requests) = mock_http_server(vec![
                (
                    200,
                    r#"{"code":0,"msg":"ok","tenant_access_token":"token","expire":7200}"#,
                ),
                (200, r#"{"code":0,"msg":"ok"}"#),
                (400, r#"{"code":131426,"msg":"invalid sequence"}"#),
            ]);
            let client = Client::with_credentials_and_config(
                AppCredentials::new("app-id", "app-secret"),
                ClientConfig::with_api_base_url(base_url),
            )
            .unwrap();
            let request =
                UpdateCardContentRequest::new("Research in progress", 7).with_uuid("content-uuid");

            client
                .update_card_element_content("card-1", "research_stream", request.clone())
                .await
                .unwrap();
            let error = client
                .update_card_element_content("card-1", "research_stream", request)
                .await
                .unwrap_err();

            assert!(matches!(error, Error::Api { .. }));
            assert_api_error(error, 131426, "invalid sequence");
            let _ = requests.recv().unwrap();
            for request in [&requests.recv().unwrap(), &requests.recv().unwrap()] {
                let (head, body) = split_request(request);
                assert!(head.starts_with(
                    "PUT /open-apis/cardkit/v1/cards/card-1/elements/research_stream/content HTTP/1.1"
                ));
                assert!(head.contains("Bearer token"));
                let body: Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["content"], "Research in progress");
                assert_eq!(body["sequence"], 7);
                assert_eq!(body["uuid"], "content-uuid");
            }
        }

        #[tokio::test]
        async fn updates_full_card_and_maps_api_errors() {
            let (base_url, requests) = mock_http_server(vec![
                (
                    200,
                    r#"{"code":0,"msg":"ok","tenant_access_token":"token","expire":7200}"#,
                ),
                (200, r#"{"code":0,"msg":"ok"}"#),
                (400, r#"{"code":131401,"msg":"card not found"}"#),
            ]);
            let client = Client::with_credentials_and_config(
                AppCredentials::new("app-id", "app-secret"),
                ClientConfig::with_api_base_url(base_url),
            )
            .unwrap();
            let card = CardBuilder::new().markdown("Final report").build().unwrap();
            let request = UpdateCardRequest::new(&card, 8)
                .unwrap()
                .with_uuid("card-uuid");

            client.update_card("card-1", request.clone()).await.unwrap();
            let error = client.update_card("card-1", request).await.unwrap_err();

            assert_api_error(error, 131401, "card not found");
            let _ = requests.recv().unwrap();
            for request in [&requests.recv().unwrap(), &requests.recv().unwrap()] {
                let (head, body) = split_request(request);
                assert!(head.starts_with("PUT /open-apis/cardkit/v1/cards/card-1 HTTP/1.1"));
                assert!(head.contains("Bearer token"));
                let body: Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["card"]["type"], "card_json");
                let data: Value =
                    serde_json::from_str(body["card"]["data"].as_str().unwrap()).unwrap();
                assert_eq!(data["schema"], "2.0");
                assert_eq!(data["body"]["elements"][0]["content"], "Final report");
                assert_eq!(body["sequence"], 8);
                assert_eq!(body["uuid"], "card-uuid");
            }
        }

        #[tokio::test]
        async fn updates_card_settings_and_maps_api_errors() {
            let (base_url, requests) = mock_http_server(vec![
                (
                    200,
                    r#"{"code":0,"msg":"ok","tenant_access_token":"token","expire":7200}"#,
                ),
                (200, r#"{"code":0,"msg":"ok"}"#),
                (400, r#"{"code":131402,"msg":"invalid settings"}"#),
            ]);
            let client = Client::with_credentials_and_config(
                AppCredentials::new("app-id", "app-secret"),
                ClientConfig::with_api_base_url(base_url),
            )
            .unwrap();
            let settings = serde_json::json!({
                "config": { "streaming_mode": false }
            });
            let request = UpdateCardSettingsRequest::new(&settings, 9)
                .unwrap()
                .with_uuid("settings-uuid");

            client
                .update_card_settings("card-1", request.clone())
                .await
                .unwrap();
            let error = client
                .update_card_settings("card-1", request)
                .await
                .unwrap_err();

            assert_api_error(error, 131402, "invalid settings");
            let _ = requests.recv().unwrap();
            for request in [&requests.recv().unwrap(), &requests.recv().unwrap()] {
                let (head, body) = split_request(request);
                assert!(
                    head.starts_with("PATCH /open-apis/cardkit/v1/cards/card-1/settings HTTP/1.1")
                );
                assert!(head.contains("Bearer token"));
                let body: Value = serde_json::from_str(body).unwrap();
                let settings: Value =
                    serde_json::from_str(body["settings"].as_str().unwrap()).unwrap();
                assert_eq!(settings["config"]["streaming_mode"], false);
                assert_eq!(body["sequence"], 9);
                assert_eq!(body["uuid"], "settings-uuid");
            }
        }

        fn assert_api_error(error: Error, code: i64, msg: &str) {
            let Error::Api {
                code: actual_code,
                msg: actual_msg,
                request_id,
            } = error
            else {
                panic!("expected an API error, got {error:?}");
            };
            assert_eq!(actual_code, code);
            assert_eq!(actual_msg, msg);
            assert_eq!(request_id.as_deref(), Some("request-id"));
        }

        fn mock_http_server(
            responses: Vec<(u16, &'static str)>,
        ) -> (String, mpsc::Receiver<String>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let (sender, receiver) = mpsc::channel::<String>();

            thread::spawn(move || {
                for (status, response) in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_request(&mut stream);
                    sender.send(request).unwrap();
                    let reason = match status {
                        200 => "OK",
                        400 => "Bad Request",
                        _ => "Status",
                    };
                    let http_response = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nX-Tt-Logid: request-id\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    );
                    stream.write_all(http_response.as_bytes()).unwrap();
                }
            });

            (base_url, receiver)
        }

        fn split_request(request: &str) -> (&str, &str) {
            let (head, body) = request.split_once("\r\n\r\n").unwrap();
            (head, body)
        }

        fn read_request(stream: &mut std::net::TcpStream) -> String {
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let size = stream.read(&mut chunk).unwrap();
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..size]);
                let text = String::from_utf8_lossy(&request).into_owned();
                let Some(header_end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let content_length = text[..header_end]
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length:"))
                    .map(str::trim)
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    return text;
                }
            }
            String::from_utf8_lossy(&request).into_owned()
        }
    }

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
        assert!(json.get("config").is_none());
        assert_eq!(json["header"]["title"]["content"], "Deployment");
        assert_eq!(json["body"]["elements"][0]["tag"], "markdown");
        assert_eq!(
            card.as_message_content().unwrap(),
            serde_json::to_string(json).unwrap()
        );
    }

    #[test]
    fn builds_streaming_card_with_element_id() {
        let card = CardBuilder::new()
            .streaming_mode(true)
            .markdown_with_id("research_stream", "Researching")
            .build()
            .unwrap();

        assert_eq!(card.as_json()["config"]["streaming_mode"], true);
        assert_eq!(
            card.as_json()["body"]["elements"][0]["element_id"],
            "research_stream"
        );
    }
}
