//! OAuth Device Flow and user access token management.

use crate::{Error, Result, auth::AppCredentials, client::ClientConfig};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEVICE_AUTHORIZATION_PATH: &str = "/open-apis/authen/v2/oauth/device_authorization";
const OAUTH_TOKEN_PATH: &str = "/open-apis/authen/v2/oauth/token";
const DEVICE_CODE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const REFRESH_TOKEN_GRANT_TYPE: &str = "refresh_token";
const MIN_POLL_INTERVAL: Duration = Duration::from_secs(1);
const SLOW_DOWN_INCREMENT: Duration = Duration::from_secs(5);

/// A user access token issued by the OAuth token endpoint.
#[derive(Clone, Serialize, Deserialize)]
pub struct UserAccessToken {
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token_expires_in: Option<u64>,
    #[serde(default)]
    token_type: String,
    #[serde(default)]
    scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    open_id: Option<String>,
}

impl UserAccessToken {
    fn from_response(response: UserTokenResponse) -> Result<Self> {
        if response.access_token.is_empty() {
            return Err(Error::InvalidResponse(
                "user access token is missing".into(),
            ));
        }
        if !response.token_type.is_empty() && response.token_type != "Bearer" {
            return Err(Error::InvalidResponse(format!(
                "unsupported OAuth token type: {}",
                response.token_type
            )));
        }

        let now = unix_now()?;
        let expires_at = (response.expires_in > 0).then(|| now + response.expires_in);
        let refresh_expires_at = response
            .refresh_token_expires_in
            .filter(|seconds| *seconds > 0)
            .map(|seconds| now + seconds);
        Ok(Self {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            expires_in: response.expires_in,
            refresh_token_expires_in: response.refresh_token_expires_in,
            token_type: response.token_type,
            scope: response.scope,
            expires_at,
            refresh_expires_at,
            open_id: response.open_id,
        })
    }

    /// Return the bearer access token.
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Return the one-time refresh token when the server issued one.
    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_deref()
    }

    /// Return the access-token lifetime in seconds.
    pub fn expires_in(&self) -> u64 {
        self.expires_in
    }

    /// Return the access-token expiry as a Unix timestamp.
    pub fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }

    /// Return the refresh-token lifetime in seconds.
    pub fn refresh_token_expires_in(&self) -> Option<u64> {
        self.refresh_token_expires_in
    }

    /// Return the refresh-token expiry as a Unix timestamp.
    pub fn refresh_expires_at(&self) -> Option<u64> {
        self.refresh_expires_at
    }

    /// Return the user's Open ID when available.
    ///
    /// The documented v2 token response does not include this field; use a
    /// caller-owned stable user identifier as the TokenStore key when it is
    /// absent.
    pub fn open_id(&self) -> Option<&str> {
        self.open_id.as_deref()
    }

    /// Return the token type, normally `Bearer`.
    pub fn token_type(&self) -> &str {
        &self.token_type
    }

    /// Return the space-separated scope string returned by the server.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Return the granted scopes as separate strings.
    pub fn scopes(&self) -> impl Iterator<Item = &str> {
        self.scope.split_whitespace()
    }

    /// Return true when the access token is valid for at least `margin`.
    pub fn is_valid(&self, margin: Duration) -> bool {
        self.expires_at.is_some_and(|expires_at| {
            unix_now().is_ok_and(|now| expires_at > now.saturating_add(margin.as_secs()))
        })
    }
}

impl std::fmt::Debug for UserAccessToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserAccessToken")
            .field("access_token", &"[redacted]")
            .field("refresh_token", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .field("refresh_expires_at", &self.refresh_expires_at)
            .field("open_id", &self.open_id)
            .field("scope", &self.scope)
            .finish()
    }
}

/// Device authorization instructions returned by the platform.
#[derive(Clone, Debug, Deserialize)]
pub struct DeviceAuthorization {
    /// URI where the user can review and enter the user code.
    pub verification_uri: String,
    /// URI that includes the user code when the platform provides one.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    /// Short code the user must confirm.
    pub user_code: String,
    /// Device grant identifier used by the token poller.
    pub device_code: String,
    /// Authorization lifetime in seconds.
    pub expires_in: u64,
    /// Recommended initial polling interval in seconds.
    #[serde(default = "default_poll_interval")]
    pub interval: u64,
}

fn default_poll_interval() -> u64 {
    5
}

/// An asynchronous user-token storage backend.
///
/// Production applications should use an encrypted or access-controlled
/// backend. The store key is the caller's stable user identifier; Feishu token
/// responses do not include `open_id`.
#[async_trait]
pub trait TokenStore: Send + Sync {
    /// Read a token for a user.
    async fn get(&self, user_id: &str) -> Result<Option<UserAccessToken>>;
    /// Save or replace a token for a user.
    async fn set(&self, user_id: &str, token: UserAccessToken) -> Result<()>;
    /// Remove a token for a user.
    async fn delete(&self, user_id: &str) -> Result<()>;
}

/// Process-local user-token store for tests and short-lived applications.
#[derive(Clone, Debug, Default)]
pub struct InMemoryTokenStore {
    tokens: Arc<tokio::sync::Mutex<HashMap<String, UserAccessToken>>>,
}

#[async_trait]
impl TokenStore for InMemoryTokenStore {
    async fn get(&self, user_id: &str) -> Result<Option<UserAccessToken>> {
        Ok(self.tokens.lock().await.get(user_id).cloned())
    }

    async fn set(&self, user_id: &str, token: UserAccessToken) -> Result<()> {
        self.tokens.lock().await.insert(user_id.to_owned(), token);
        Ok(())
    }

    async fn delete(&self, user_id: &str) -> Result<()> {
        self.tokens.lock().await.remove(user_id);
        Ok(())
    }
}

/// Feishu OAuth Device Flow client.
pub struct DeviceFlowClient {
    credentials: AppCredentials,
    config: ClientConfig,
    http: reqwest::Client,
}

impl DeviceFlowClient {
    /// Create a Device Flow client.
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
            http: reqwest::Client::builder()
                .build()
                .map_err(|error| Error::Config(format!("failed to create HTTP client: {error}")))?,
        })
    }

    /// Start Device Flow authorization for the requested scopes.
    ///
    /// The application is responsible for showing `verification_uri_complete`
    /// or `verification_uri` and `user_code` to the user. Device Flow needs no
    /// redirect URI or HTTP callback.
    pub async fn start<S, T>(&self, scopes: S) -> Result<DeviceAuthorization>
    where
        S: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let scope = scopes
            .into_iter()
            .map(|scope| scope.as_ref().to_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let body = DeviceAuthorizationRequest {
            client_id: self.credentials.app_id.clone(),
            scope,
        };
        let response = self
            .http
            .post(self.config.api_url(DEVICE_AUTHORIZATION_PATH))
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("X-Tt-Logid")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        let envelope: DeviceAuthorizationResponse = serde_json::from_slice(&bytes)?;
        if !status.is_success() || envelope.code != 0 {
            return Err(Error::Api {
                code: envelope.code,
                msg: envelope
                    .msg
                    .unwrap_or_else(|| "device authorization failed".into()),
                request_id,
            });
        }

        envelope
            .data
            .ok_or_else(|| Error::InvalidResponse("device authorization data is missing".into()))
    }

    /// Poll once for an access token.
    pub async fn poll_once(&self, device_code: impl AsRef<str>) -> Result<UserAccessToken> {
        let body = UserTokenRequest {
            client_id: self.credentials.app_id.clone(),
            client_secret: self.credentials.app_secret.clone(),
            grant_type: DEVICE_CODE_GRANT_TYPE,
            device_code: device_code.as_ref().to_owned(),
        };
        self.post_token_request(body).await
    }

    /// Poll until authorization completes, fails, or times out.
    pub async fn poll(
        &self,
        device_code: impl AsRef<str>,
        interval: Option<Duration>,
        timeout: Option<Duration>,
    ) -> Result<UserAccessToken> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut interval = interval.unwrap_or(MIN_POLL_INTERVAL).max(MIN_POLL_INTERVAL);

        loop {
            match self.poll_once(&device_code).await {
                Ok(token) => return Ok(token),
                Err(Error::OAuth { error, .. }) if error == "authorization_pending" => {}
                Err(Error::OAuth { error, .. }) if error == "slow_down" => {
                    interval = interval.saturating_add(SLOW_DOWN_INCREMENT);
                }
                Err(error) => return Err(error),
            }

            let next_at = Instant::now() + interval;
            if deadline.is_some_and(|deadline| next_at > deadline) {
                return Err(Error::OAuth {
                    code: 0,
                    error: "expired_token".into(),
                    error_description: "device authorization timed out".into(),
                    request_id: None,
                });
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Refresh a user access token.
    ///
    /// A refresh token is single-use. When the response contains a new refresh
    /// token, the caller must persist it immediately.
    pub async fn refresh(&self, refresh_token: impl AsRef<str>) -> Result<UserAccessToken> {
        let body = RefreshTokenRequest {
            client_id: self.credentials.app_id.clone(),
            client_secret: self.credentials.app_secret.clone(),
            grant_type: REFRESH_TOKEN_GRANT_TYPE,
            refresh_token: refresh_token.as_ref().to_owned(),
        };
        self.post_token_request(body).await
    }

    async fn post_token_request<B>(&self, body: B) -> Result<UserAccessToken>
    where
        B: serde::Serialize,
    {
        let response = self
            .http
            .post(self.config.api_url(OAUTH_TOKEN_PATH))
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("X-Tt-Logid")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        let token: UserTokenResponse = serde_json::from_slice(&bytes)?;
        if !status.is_success() || token.code != 0 || token.error.is_some() {
            return Err(Error::OAuth {
                code: token.code,
                error: token.error.unwrap_or_else(|| "server_error".into()),
                error_description: token
                    .error_description
                    .or(token.msg)
                    .unwrap_or_else(|| "OAuth token request failed".into()),
                request_id,
            });
        }

        UserAccessToken::from_response(token)
    }
}

#[derive(Serialize)]
struct DeviceAuthorizationRequest {
    client_id: String,
    scope: String,
}

#[derive(Deserialize)]
struct DeviceAuthorizationResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    data: Option<DeviceAuthorization>,
}

#[derive(Serialize)]
struct UserTokenRequest {
    client_id: String,
    client_secret: String,
    grant_type: &'static str,
    device_code: String,
}

#[derive(Serialize)]
struct RefreshTokenRequest {
    client_id: String,
    client_secret: String,
    grant_type: &'static str,
    refresh_token: String,
}

#[derive(Deserialize)]
struct UserTokenResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    refresh_token_expires_in: Option<u64>,
    #[serde(default)]
    token_type: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    open_id: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

fn unix_now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| Error::InvalidResponse("system clock is before Unix epoch".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[tokio::test]
    async fn starts_device_authorization_with_space_separated_scopes() {
        let (base_url, requests) = mock_http_server(vec![(
            200,
            r#"{
                "code": 0,
                "msg": "ok",
                "data": {
                    "verification_uri": "https://example.com/device",
                    "verification_uri_complete": "https://example.com/device?code=USER",
                    "user_code": "USER",
                    "device_code": "DEVICE",
                    "expires_in": 600,
                    "interval": 5
                }
            }"#,
        )]);
        let client = DeviceFlowClient::new(
            AppCredentials::new("app-id", "app-secret"),
            ClientConfig::with_api_base_url(base_url),
        )
        .unwrap();

        let authorization = client
            .start(["im:message", "offline_access"])
            .await
            .unwrap();

        assert_eq!(authorization.verification_uri, "https://example.com/device");
        assert_eq!(
            authorization.verification_uri_complete.as_deref(),
            Some("https://example.com/device?code=USER")
        );
        assert_eq!(authorization.user_code, "USER");
        assert_eq!(authorization.device_code, "DEVICE");
        assert_eq!(authorization.expires_in, 600);
        assert_eq!(authorization.interval, 5);
        let request = requests.recv().unwrap();
        let (head, body) = split_request(&request);
        assert!(head.starts_with("POST /open-apis/authen/v2/oauth/device_authorization HTTP/1.1"));
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["client_id"], "app-id");
        assert_eq!(body["scope"], "im:message offline_access");
    }

    #[tokio::test]
    async fn polls_until_authorization_succeeds() {
        let (base_url, requests) = mock_http_server(vec![
            (
                400,
                r#"{"code":0,"error":"authorization_pending","error_description":"pending"}"#,
            ),
            (
                200,
                r#"{
                    "code": 0,
                    "access_token": "user-token",
                    "expires_in": 7200,
                    "refresh_token": "refresh-token",
                    "refresh_token_expires_in": 604800,
                    "token_type": "Bearer",
                    "scope": "im:message offline_access",
                    "open_id": "ou_user"
                }"#,
            ),
        ]);
        let client = DeviceFlowClient::new(
            AppCredentials::new("app-id", "app-secret"),
            ClientConfig::with_api_base_url(base_url),
        )
        .unwrap();

        let token = client
            .poll(
                "DEVICE",
                Some(Duration::from_secs(1)),
                Some(Duration::from_secs(5)),
            )
            .await
            .unwrap();

        assert_eq!(token.access_token(), "user-token");
        assert_eq!(token.refresh_token(), Some("refresh-token"));
        assert_eq!(token.expires_in(), 7200);
        assert_eq!(token.refresh_token_expires_in(), Some(604800));
        assert_eq!(token.token_type(), "Bearer");
        assert_eq!(token.open_id(), Some("ou_user"));
        assert_eq!(
            token.scopes().collect::<Vec<_>>(),
            vec!["im:message", "offline_access"]
        );
        assert!(token.is_valid(Duration::from_secs(300)));
        let _ = requests.recv().unwrap();
        let request = requests.recv().unwrap();
        let (head, body) = split_request(&request);
        assert!(head.starts_with("POST /open-apis/authen/v2/oauth/token HTTP/1.1"));
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["client_id"], "app-id");
        assert_eq!(body["client_secret"], "app-secret");
        assert_eq!(
            body["grant_type"],
            "urn:ietf:params:oauth:grant-type:device_code"
        );
        assert_eq!(body["device_code"], "DEVICE");
    }

    #[tokio::test]
    async fn handles_slow_down_and_timeout() {
        let (base_url, requests) = mock_http_server(vec![(
            400,
            r#"{"code":0,"error":"slow_down","error_description":"slow down"}"#,
        )]);
        let client = DeviceFlowClient::new(
            AppCredentials::new("app-id", "app-secret"),
            ClientConfig::with_api_base_url(base_url),
        )
        .unwrap();

        let error = client
            .poll(
                "DEVICE",
                Some(Duration::from_secs(1)),
                Some(Duration::from_secs(2)),
            )
            .await
            .unwrap_err();

        assert_oauth_error(&error, 0, "expired_token", "device authorization timed out");
        let request = requests.recv().unwrap();
        assert!(request.contains(r#""grant_type":"urn:ietf:params:oauth:grant-type:device_code""#));
        assert!(request.contains(r#""device_code":"DEVICE""#));
    }

    #[tokio::test]
    async fn handles_denied_and_expired_authorization() {
        for (error_type, description) in [
            ("access_denied", "The user denied the request."),
            ("expired_token", "The device code has expired."),
        ] {
            let response = format!(
                r#"{{"code":0,"error":"{error_type}","error_description":"{description}"}}"#
            );
            let (base_url, _requests) =
                mock_http_server(vec![(400, Box::leak(response.into_boxed_str()))]);
            let client = DeviceFlowClient::new(
                AppCredentials::new("app-id", "app-secret"),
                ClientConfig::with_api_base_url(base_url),
            )
            .unwrap();

            let error = client.poll_once("DEVICE").await.unwrap_err();
            assert_oauth_error(&error, 0, error_type, description);
        }
    }

    #[tokio::test]
    async fn refreshes_and_rotates_refresh_token() {
        let (base_url, requests) = mock_http_server(vec![(
            200,
            r#"{
                "code": 0,
                "access_token": "new-user-token",
                "expires_in": 7200,
                "refresh_token": "new-refresh-token",
                "refresh_token_expires_in": 604800,
                "token_type": "Bearer",
                "scope": "im:message offline_access"
            }"#,
        )]);
        let client = DeviceFlowClient::new(
            AppCredentials::new("app-id", "app-secret"),
            ClientConfig::with_api_base_url(base_url),
        )
        .unwrap();

        let token = client.refresh("old-refresh-token").await.unwrap();

        assert_eq!(token.access_token(), "new-user-token");
        assert_eq!(token.refresh_token(), Some("new-refresh-token"));
        let request = requests.recv().unwrap();
        let (head, body) = split_request(&request);
        assert!(head.starts_with("POST /open-apis/authen/v2/oauth/token HTTP/1.1"));
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["client_id"], "app-id");
        assert_eq!(body["client_secret"], "app-secret");
        assert_eq!(body["grant_type"], "refresh_token");
        assert_eq!(body["refresh_token"], "old-refresh-token");
    }

    #[tokio::test]
    async fn maps_refresh_errors_with_request_id() {
        let (base_url, _requests) = mock_http_server(vec![(
            400,
            r#"{"code":20064,"error":"invalid_grant","error_description":"revoked"}"#,
        )]);
        let client = DeviceFlowClient::new(
            AppCredentials::new("app-id", "app-secret"),
            ClientConfig::with_api_base_url(base_url),
        )
        .unwrap();

        let error = client.refresh("old-refresh-token").await.unwrap_err();

        assert_oauth_error(&error, 20064, "invalid_grant", "revoked");
        assert!(matches!(
            error,
            Error::OAuth {
                request_id: Some(ref id),
                ..
            } if id == "request-id"
        ));
    }

    #[tokio::test]
    async fn memory_token_store_manages_tokens() {
        let (base_url, _requests) = mock_http_server(vec![(
            200,
            r#"{
                "code": 0,
                "access_token": "user-token",
                "expires_in": 7200,
                "refresh_token": "refresh-token",
                "refresh_token_expires_in": 604800,
                "token_type": "Bearer",
                "scope": "im:message offline_access"
            }"#,
        )]);
        let client = DeviceFlowClient::new(
            AppCredentials::new("app-id", "app-secret"),
            ClientConfig::with_api_base_url(base_url),
        )
        .unwrap();
        let token = client.refresh("old-refresh-token").await.unwrap();
        let store = InMemoryTokenStore::default();

        store.set("ou_user", token.clone()).await.unwrap();
        let debug = format!("{token:?}");
        assert!(!debug.contains("user-token"));
        assert!(!debug.contains("refresh-token"));
        assert_eq!(
            store.get("ou_user").await.unwrap().unwrap().access_token(),
            "user-token"
        );
        store.delete("ou_user").await.unwrap();
        assert!(store.get("ou_user").await.unwrap().is_none());
    }

    fn assert_oauth_error(error: &Error, code: i64, error_type: &str, description: &str) {
        let Error::OAuth {
            code: actual_code,
            error,
            error_description,
            request_id: _,
        } = error
        else {
            panic!("expected OAuth error, got {error:?}");
        };
        assert_eq!(*actual_code, code);
        assert_eq!(error, error_type);
        assert_eq!(error_description, description);
    }

    fn mock_http_server(responses: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel::<String>();

        std::thread::spawn(move || {
            for (status, response) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                sender.send(request).unwrap();
                let reason = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    _ => "Status",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nX-Tt-Logid: request-id\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        (base_url, receiver)
    }

    fn split_request(request: &str) -> (&str, &str) {
        request.split_once("\r\n\r\n").unwrap()
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
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
                        .map(|(_, value)| value)
                })
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
