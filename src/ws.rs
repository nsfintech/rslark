//! Feishu and Lark WebSocket long connections.

mod proto;

use crate::{Error, Result, auth::AppCredentials, client::ClientConfig, events::RawEvent};
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::interval;
use tokio_tungstenite::connect_async;
use url::Url;

const BOOTSTRAP_PATH: &str = "/callback/ws/endpoint";
const CONTROL_FRAME: i32 = 0;
const DATA_FRAME: i32 = 1;
const HEADER_TYPE: &str = "type";
const HEADER_SUM: &str = "sum";
const HEADER_SEQ: &str = "seq";
const HEADER_MESSAGE_ID: &str = "message_id";
const HEADER_BIZ_RT: &str = "biz_rt";
const PONG_GRACE_PERIOD: Duration = Duration::from_secs(5);

#[derive(Serialize)]
struct BootstrapRequest {
    #[serde(rename = "AppID")]
    app_id: String,
    #[serde(rename = "AppSecret")]
    app_secret: String,
}

#[derive(Deserialize)]
struct BootstrapResponse {
    code: i64,
    msg: String,
    data: Option<BootstrapEndpoint>,
}

#[derive(Deserialize)]
struct BootstrapEndpoint {
    #[serde(rename = "URL")]
    url: String,
    #[serde(rename = "ClientConfig")]
    client_config: Option<ServerConfig>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
struct ServerConfig {
    #[serde(rename = "ReconnectCount", default)]
    reconnect_count: Option<i64>,
    #[serde(rename = "ReconnectInterval", default)]
    reconnect_interval: Option<u64>,
    #[serde(rename = "ReconnectNonce", default)]
    reconnect_nonce: Option<u64>,
    #[serde(rename = "PingInterval", default)]
    ping_interval: Option<u64>,
}

impl ServerConfig {
    fn reconnect_count(self) -> i64 {
        self.reconnect_count.unwrap_or(-1)
    }

    fn reconnect_interval(self) -> Duration {
        Duration::from_secs(
            self.reconnect_interval
                .filter(|seconds| *seconds > 0)
                .unwrap_or(120),
        )
        .max(Duration::from_secs(1))
    }

    fn reconnect_nonce(self) -> u64 {
        self.reconnect_nonce
            .filter(|seconds| *seconds > 0)
            .unwrap_or(30)
    }

    fn ping_interval(self) -> Duration {
        Duration::from_secs(self.ping_interval.unwrap_or(120).max(1))
    }

    fn read_timeout(self) -> Duration {
        self.ping_interval()
            .saturating_mul(2)
            .saturating_add(PONG_GRACE_PERIOD)
    }

    fn merge(&mut self, newer: Self) {
        self.reconnect_count = newer.reconnect_count.or(self.reconnect_count);
        self.reconnect_interval = newer.reconnect_interval.or(self.reconnect_interval);
        self.reconnect_nonce = newer.reconnect_nonce.or(self.reconnect_nonce);
        self.ping_interval = newer.ping_interval.or(self.ping_interval);
    }
}

#[derive(Serialize)]
struct EventResponse {
    code: u16,
    headers: HashMap<String, String>,
    data: Option<serde_json::Value>,
}

/// A WebSocket long-connection client.
#[derive(Clone)]
pub struct WsClient {
    credentials: AppCredentials,
    config: ClientConfig,
    router: crate::events::EventRouter,
    auto_reconnect: bool,
    http: reqwest::Client,
}

impl WsClient {
    /// Create a client for a self-built application.
    pub fn new(credentials: AppCredentials, config: ClientConfig) -> Result<Self> {
        if credentials.app_id.trim().is_empty() || credentials.app_secret.trim().is_empty() {
            return Err(Error::Config(
                "app_id and app_secret cannot be empty".into(),
            ));
        }
        if config.api_base_url.trim().is_empty() {
            return Err(Error::Config("api_base_url cannot be empty".into()));
        }

        Ok(Self {
            credentials,
            config,
            router: crate::events::EventRouter::new(),
            auto_reconnect: true,
            http: reqwest::Client::builder()
                .build()
                .map_err(|error| Error::Config(format!("failed to create HTTP client: {error}")))?,
        })
    }

    /// Set the event router used by the long connection.
    pub fn with_router(mut self, router: crate::events::EventRouter) -> Self {
        self.router = router;
        self
    }

    /// Enable or disable automatic reconnection.
    pub fn with_auto_reconnect(mut self, enabled: bool) -> Self {
        self.auto_reconnect = enabled;
        self
    }

    /// Run the connection until it fails, the caller cancels the future, or
    /// automatic reconnection is disabled after a disconnect.
    pub async fn run(&self) -> Result<()> {
        let mut server_config = ServerConfig::default();
        let mut attempts: i64 = 0;

        loop {
            let mut ever_connected = false;
            let result = self
                .connect_once(&mut server_config, &mut ever_connected)
                .await;
            if ever_connected {
                attempts = 0;
            }
            if let Err(error) = result {
                if !self.auto_reconnect {
                    return Err(error);
                }
            }

            let max_attempts = server_config.reconnect_count();
            if max_attempts >= 0 && attempts >= max_attempts {
                return Err(Error::InvalidResponse(format!(
                    "WebSocket reconnect attempts exhausted after {attempts}"
                )));
            }

            let delay = if attempts == 0 {
                let nonce = server_config.reconnect_nonce();
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|value| value.subsec_nanos() as u64)
                    .unwrap_or(0);
                Duration::from_secs(nanos % nonce)
            } else {
                server_config.reconnect_interval()
            };
            tokio::time::sleep(delay).await;
            attempts += 1;
        }
    }

    async fn connect_once(
        &self,
        server_config: &mut ServerConfig,
        ever_connected: &mut bool,
    ) -> Result<()> {
        let endpoint = self.fetch_endpoint(server_config).await?;
        let parsed = Url::parse(&endpoint.url)
            .map_err(|error| Error::Config(format!("invalid WebSocket URL: {error}")))?;
        let service_id = parsed
            .query_pairs()
            .find(|(key, _)| key == "service_id")
            .and_then(|(_, value)| value.parse::<i32>().ok())
            .unwrap_or_default();

        let (mut socket, response) = connect_async(endpoint.url.as_str()).await?;
        if response.status().as_u16() != 101 {
            return Err(Error::InvalidResponse(format!(
                "WebSocket handshake returned HTTP {}",
                response.status()
            )));
        }
        *ever_connected = true;

        let ping_interval = server_config.ping_interval();
        let mut ping_timer = interval(ping_interval);
        ping_timer.tick().await;
        let read_deadline = tokio::time::sleep(server_config.read_timeout());
        tokio::pin!(read_deadline);
        let mut fragments: HashMap<String, HashMap<usize, Vec<u8>>> = HashMap::new();

        loop {
            tokio::select! {
                _ = ping_timer.tick() => {
                    let ping = proto::Frame {
                        service: service_id,
                        method: CONTROL_FRAME,
                        headers: vec![proto::Header {
                            key: HEADER_TYPE.into(),
                            value: "ping".into(),
                        }],
                        ..proto::Frame::default()
                    };
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                            ping.encode_to_vec().into(),
                        ))
                        .await?;
                }
                _ = &mut read_deadline => {
                    return Err(Error::InvalidResponse(
                        "WebSocket read timed out waiting for a pong or event".into(),
                    ));
                }
                message = socket.next() => {
                    let Some(message) = message else {
                        return Err(Error::InvalidResponse("WebSocket closed by server".into()));
                    };
                    let message = message?;
                    match message {
                        tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                            if let Some(response) = self.handle_frame(
                                bytes.as_ref(),
                                &mut fragments,
                                server_config,
                            )? {
                                socket
                                    .send(tokio_tungstenite::tungstenite::Message::Binary(
                                        response.encode_to_vec().into(),
                                    ))
                                    .await?;
                            }
                        }
                        tokio_tungstenite::tungstenite::Message::Close(_) => {
                            return Err(Error::InvalidResponse(
                                "WebSocket closed by server".into(),
                            ));
                        }
                        _ => {}
                    }
                    read_deadline
                        .as_mut()
                        .reset(tokio::time::Instant::now() + server_config.read_timeout());
                }
            }
        }
    }

    async fn fetch_endpoint(&self, server_config: &mut ServerConfig) -> Result<BootstrapEndpoint> {
        let url = self.config.api_url(BOOTSTRAP_PATH);
        let response = self
            .http
            .post(url)
            .header("locale", "zh")
            .json(&BootstrapRequest {
                app_id: self.credentials.app_id.clone(),
                app_secret: self.credentials.app_secret.clone(),
            })
            .send()
            .await?;
        let status = response.status();
        let body = response.bytes().await?;
        let bootstrap: BootstrapResponse = serde_json::from_slice(&body).map_err(Error::from)?;
        if !status.is_success() || bootstrap.code != 0 {
            return Err(Error::Api {
                code: bootstrap.code,
                msg: bootstrap.msg,
                request_id: None,
            });
        }
        let endpoint = bootstrap
            .data
            .ok_or_else(|| Error::InvalidResponse("WebSocket endpoint is missing".into()))?;
        if let Some(config) = endpoint.client_config {
            server_config.merge(config);
        }
        Ok(endpoint)
    }

    fn handle_frame(
        &self,
        bytes: &[u8],
        fragments: &mut HashMap<String, HashMap<usize, Vec<u8>>>,
        server_config: &mut ServerConfig,
    ) -> Result<Option<proto::Frame>> {
        let frame = proto::Frame::decode(bytes)
            .map_err(|error| Error::InvalidResponse(format!("invalid WebSocket frame: {error}")))?;
        if frame.method == CONTROL_FRAME {
            if header_value(&frame.headers, HEADER_TYPE).as_deref() == Some("pong") {
                if let Some(payload) = &frame.payload {
                    if let Ok(config) = serde_json::from_slice::<ServerConfig>(payload) {
                        server_config.merge(config);
                    }
                }
            }
            return Ok(None);
        }

        if frame.method != DATA_FRAME {
            return Ok(None);
        }
        let sum = header_value(&frame.headers, HEADER_SUM)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        let seq = header_value(&frame.headers, HEADER_SEQ)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let message_id = header_value(&frame.headers, HEADER_MESSAGE_ID).unwrap_or_default();
        let payload = if sum <= 1 {
            frame.payload.clone().unwrap_or_default()
        } else {
            let chunks = fragments.entry(message_id.clone()).or_default();
            chunks.insert(seq, frame.payload.clone().unwrap_or_default());
            if chunks.len() != sum {
                return Ok(None);
            }
            let mut combined = Vec::new();
            for index in 0..sum {
                let Some(chunk) = chunks.remove(&index) else {
                    fragments.remove(&message_id);
                    return Ok(None);
                };
                combined.extend_from_slice(&chunk);
            }
            fragments.remove(&message_id);
            combined
        };

        let message_type = header_value(&frame.headers, HEADER_TYPE).unwrap_or_default();
        if message_type != "event" && message_type != "card" {
            return Ok(None);
        }
        let payload: serde_json::Value = serde_json::from_slice(&payload)
            .map_err(|error| Error::InvalidResponse(format!("invalid event JSON: {error}")))?;
        let event = RawEvent::from_payload(payload);
        let started = SystemTime::now();
        let dispatch_result = self.router.dispatch(&event);
        let elapsed = SystemTime::now()
            .duration_since(started)
            .map(|value| value.as_millis().to_string())
            .unwrap_or_else(|_| "0".into());

        let mut headers = frame.headers.clone();
        set_header(&mut headers, HEADER_BIZ_RT, elapsed);
        let response = EventResponse {
            code: if dispatch_result.is_ok() { 200 } else { 500 },
            headers: HashMap::new(),
            data: dispatch_result.unwrap_or(None),
        };
        let mut response_frame = frame;
        response_frame.headers = headers;
        response_frame.payload = Some(serde_json::to_vec(&response)?);
        Ok(Some(response_frame))
    }
}

fn header_value(headers: &[proto::Header], key: &str) -> Option<String> {
    headers
        .iter()
        .find(|header| header.key == key)
        .map(|header| header.value.clone())
}

fn set_header(headers: &mut Vec<proto::Header>, key: &str, value: String) {
    if let Some(header) = headers.iter_mut().find(|header| header.key == key) {
        header.value = value;
    } else {
        headers.push(proto::Header {
            key: key.into(),
            value,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{auth::AppCredentials, events::EventRouter};
    use prost::Message;
    use std::sync::{Arc, Mutex};

    #[test]
    fn normalizes_server_reconnect_and_ping_settings() {
        let config = ServerConfig {
            reconnect_count: Some(-1),
            reconnect_interval: Some(0),
            reconnect_nonce: Some(0),
            ping_interval: Some(0),
        };

        assert_eq!(config.reconnect_count(), -1);
        assert_eq!(config.reconnect_interval(), Duration::from_secs(120));
        assert_eq!(config.reconnect_nonce(), 30);
        assert_eq!(config.ping_interval(), Duration::from_secs(1));
        assert_eq!(config.read_timeout(), Duration::from_secs(7));
    }

    #[test]
    fn reassembles_fragments_and_writes_ack() {
        let received = Arc::new(Mutex::new(None));
        let handler_received = Arc::clone(&received);
        let router = EventRouter::new().on("test.event", move |event| {
            *handler_received.lock().unwrap() = Some(event.payload.clone());
            Ok(None)
        });
        let client = WsClient::new(
            AppCredentials::new("app-id", "app-secret"),
            ClientConfig::with_api_base_url("http://localhost"),
        )
        .unwrap()
        .with_router(router);
        let mut server_config = ServerConfig::default();
        let mut fragments = HashMap::new();

        let first = proto::Frame {
            method: DATA_FRAME,
            headers: fragment_headers("message-1", 2, 0),
            payload: Some(br#"{"header":{"event_type":"test.event"},"#.to_vec()),
            ..proto::Frame::default()
        };
        assert!(
            client
                .handle_frame(&first.encode_to_vec(), &mut fragments, &mut server_config)
                .unwrap()
                .is_none()
        );

        let second = proto::Frame {
            method: DATA_FRAME,
            headers: fragment_headers("message-1", 2, 1),
            payload: Some(br#""event":{"value":1}}"#.to_vec()),
            ..proto::Frame::default()
        };
        let ack = client
            .handle_frame(&second.encode_to_vec(), &mut fragments, &mut server_config)
            .unwrap()
            .unwrap();

        let event = received.lock().unwrap().take().unwrap();
        assert_eq!(event["event"]["value"], 1);
        let ack_payload: serde_json::Value = serde_json::from_slice(&ack.payload.unwrap()).unwrap();
        assert_eq!(ack_payload["code"], 200);
    }

    fn fragment_headers(message_id: &str, sum: usize, seq: usize) -> Vec<proto::Header> {
        vec![
            proto::Header {
                key: HEADER_TYPE.into(),
                value: "event".into(),
            },
            proto::Header {
                key: HEADER_MESSAGE_ID.into(),
                value: message_id.into(),
            },
            proto::Header {
                key: HEADER_SUM.into(),
                value: sum.to_string(),
            },
            proto::Header {
                key: HEADER_SEQ.into(),
                value: seq.to_string(),
            },
        ]
    }
}
