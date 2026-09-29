//! Instant messaging API requests and responses.

use crate::{Result, client::Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Identifier type used to address a message receiver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveIdType {
    OpenId,
    UserId,
    UnionId,
    Email,
    ChatId,
}

impl ReceiveIdType {
    /// Platform query value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenId => "open_id",
            Self::UserId => "user_id",
            Self::UnionId => "union_id",
            Self::Email => "email",
            Self::ChatId => "chat_id",
        }
    }
}

/// Request body for creating a message.
#[derive(Debug, Serialize)]
pub struct SendMessageRequest {
    pub receive_id: String,
    pub msg_type: String,
    /// Platform message content represented as serialized JSON.
    pub content: String,
    pub uuid: Option<String>,
}

/// Request body for replying to a message.
#[derive(Debug, Serialize)]
pub struct ReplyMessageRequest {
    pub msg_type: String,
    /// Platform message content represented as serialized JSON.
    pub content: String,
    pub reply_in_thread: Option<bool>,
    pub uuid: Option<String>,
}

/// A message returned by the platform.
#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_app_link: Option<String>,
}

impl Client {
    /// Create a message with arbitrary platform message JSON.
    pub async fn send_message(
        &self,
        receive_id_type: ReceiveIdType,
        request: SendMessageRequest,
    ) -> Result<Message> {
        self.request_json(
            reqwest::Method::POST,
            "/open-apis/im/v1/messages",
            Some(vec![(
                "receive_id_type",
                receive_id_type.as_str().to_owned(),
            )]),
            Some(&request),
        )
        .await
    }

    /// Send a text message.
    pub async fn send_text(
        &self,
        receive_id_type: ReceiveIdType,
        receive_id: impl Into<String>,
        text: impl AsRef<str>,
        uuid: Option<String>,
    ) -> Result<Message> {
        self.send_message(
            receive_id_type,
            SendMessageRequest {
                receive_id: receive_id.into(),
                msg_type: "text".into(),
                content: serde_json::json!({ "text": text.as_ref() }).to_string(),
                uuid,
            },
        )
        .await
    }

    /// Send an interactive card represented by Card JSON.
    pub async fn send_card(
        &self,
        receive_id_type: ReceiveIdType,
        receive_id: impl Into<String>,
        card: &crate::card::Card,
        uuid: Option<String>,
    ) -> Result<Message> {
        self.send_message(
            receive_id_type,
            SendMessageRequest {
                receive_id: receive_id.into(),
                msg_type: "interactive".into(),
                content: card.as_message_content()?,
                uuid,
            },
        )
        .await
    }

    /// Reply to a message with arbitrary platform message JSON.
    pub async fn reply_message(
        &self,
        message_id: impl AsRef<str>,
        request: ReplyMessageRequest,
    ) -> Result<Message> {
        self.request_json(
            reqwest::Method::POST,
            &format!("/open-apis/im/v1/messages/{}/reply", message_id.as_ref()),
            None,
            Some(&request),
        )
        .await
    }

    /// Reply to a message with text.
    pub async fn reply_text(
        &self,
        message_id: impl AsRef<str>,
        text: impl AsRef<str>,
        reply_in_thread: Option<bool>,
        uuid: Option<String>,
    ) -> Result<Message> {
        self.reply_message(
            message_id,
            ReplyMessageRequest {
                msg_type: "text".into(),
                content: serde_json::json!({ "text": text.as_ref() }).to_string(),
                reply_in_thread,
                uuid,
            },
        )
        .await
    }

    /// Get a message by its platform identifier.
    pub async fn get_message(&self, message_id: impl AsRef<str>) -> Result<Message> {
        self.request_json(
            reqwest::Method::GET,
            &format!("/open-apis/im/v1/messages/{}", message_id.as_ref()),
            None,
            None::<&Value>,
        )
        .await
    }

    /// Recall a message.
    pub async fn delete_message(&self, message_id: impl AsRef<str>) -> Result<()> {
        self.request_empty(
            reqwest::Method::DELETE,
            &format!("/open-apis/im/v1/messages/{}", message_id.as_ref()),
            None,
            None::<&Value>,
        )
        .await
    }
}

#[cfg(all(test, feature = "http"))]
mod tests {
    use crate::{auth::AppCredentials, client::ClientConfig, im::ReceiveIdType};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    #[tokio::test]
    async fn sends_message_with_cached_tenant_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (request_sender, request_receiver) = mpsc::channel::<String>();

        thread::spawn(move || {
            let responses = [
                r#"{"code":0,"msg":"ok","tenant_access_token":"tenant-token","expire":7200}"#,
                r#"{"code":0,"msg":"ok","data":{"message_id":"om_1","msg_type":"text"}}"#,
            ];
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                request_sender.send(request).unwrap();
                let http_response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                stream.write_all(http_response.as_bytes()).unwrap();
            }
        });

        let client = crate::Client::with_credentials_and_config(
            AppCredentials::new("app-id", "app-secret"),
            ClientConfig::with_api_base_url(base_url),
        )
        .unwrap();
        let message = client
            .send_text(
                ReceiveIdType::ChatId,
                "oc_1",
                "hello",
                Some("uuid-1".into()),
            )
            .await
            .unwrap();
        assert_eq!(message.message_id.as_deref(), Some("om_1"));

        let token_request = request_receiver.recv().unwrap();
        let message_request = request_receiver.recv().unwrap();
        assert!(token_request.starts_with("POST /open-apis/auth/v3/tenant_access_token/internal"));
        assert!(token_request.contains(r#""app_id":"app-id""#));
        assert!(token_request.contains(r#""app_secret":"app-secret""#));
        assert!(
            message_request.starts_with("POST /open-apis/im/v1/messages?receive_id_type=chat_id")
        );
        assert!(message_request.contains("Bearer tenant-token"));
        assert!(message_request.contains(r#""receive_id":"oc_1""#));
        assert!(message_request.contains(r#""content":"{\"text\":\"hello\"}""#));
        assert!(message_request.contains(r#""uuid":"uuid-1""#));
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
