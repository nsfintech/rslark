//! Client configuration and authenticated Open API transport.

#[cfg(feature = "http")]
use crate::auth::{CachedTenantToken, TenantTokenRequest, TenantTokenResponse};
use crate::{Error, Result, auth::AppCredentials};
#[cfg(feature = "http")]
use serde::de::DeserializeOwned;
#[cfg(feature = "http")]
use std::sync::Arc;
#[cfg(feature = "http")]
const TENANT_TOKEN_PATH: &str = "/open-apis/auth/v3/tenant_access_token/internal";
#[cfg(feature = "http")]
const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
#[cfg(feature = "http")]
use std::time::Duration;

/// Configuration shared by the SDK client modules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientConfig {
    /// Base URL for the Feishu or Lark API.
    pub api_base_url: String,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self::feishu()
    }
}

impl ClientConfig {
    /// Use the Feishu API endpoint.
    pub fn feishu() -> Self {
        Self::with_api_base_url("https://open.feishu.cn")
    }

    /// Use the Lark API endpoint.
    pub fn lark() -> Self {
        Self::with_api_base_url("https://open.larksuite.com")
    }

    /// Configure an API endpoint, including a self-hosted compatible endpoint.
    pub fn with_api_base_url(url: impl Into<String>) -> Self {
        Self {
            api_base_url: url.into(),
        }
    }

    #[cfg(any(feature = "http", feature = "websocket"))]
    pub(crate) fn api_url(&self, path: &str) -> String {
        format!("{}/", self.api_base_url.trim_end_matches('/')) + path.trim_start_matches('/')
    }
}

/// Top-level entry point for SDK functionality.
#[derive(Clone, Debug)]
pub struct Client {
    config: ClientConfig,
    #[cfg(feature = "http")]
    credentials: Option<AppCredentials>,
    #[cfg(feature = "http")]
    http: reqwest::Client,
    #[cfg(feature = "http")]
    token_cache: Arc<tokio::sync::Mutex<Option<CachedTenantToken>>>,
}

impl Client {
    /// Create an unauthenticated client with the default Feishu configuration.
    pub fn new() -> Result<Self> {
        Self::with_config(ClientConfig::default())
    }

    /// Create a client with explicit configuration.
    pub fn with_config(config: ClientConfig) -> Result<Self> {
        Ok(Self {
            config,
            #[cfg(feature = "http")]
            credentials: None,
            #[cfg(feature = "http")]
            http: build_http_client()?,
            #[cfg(feature = "http")]
            token_cache: Arc::default(),
        })
    }

    /// Create a client for a self-built Feishu application.
    pub fn with_app_credentials(credentials: AppCredentials) -> Result<Self> {
        Self::with_credentials_and_config(credentials, ClientConfig::feishu())
    }

    /// Create a client for a self-built Lark application.
    pub fn with_lark_credentials(credentials: AppCredentials) -> Result<Self> {
        Self::with_credentials_and_config(credentials, ClientConfig::lark())
    }

    /// Create a client with explicit credentials and configuration.
    pub fn with_credentials_and_config(
        credentials: AppCredentials,
        config: ClientConfig,
    ) -> Result<Self> {
        if credentials.app_id.trim().is_empty() || credentials.app_secret.trim().is_empty() {
            return Err(Error::Config(
                "app_id and app_secret cannot be empty".into(),
            ));
        }
        if config.api_base_url.trim().is_empty() {
            return Err(Error::Config("api_base_url cannot be empty".into()));
        }

        Ok(Self {
            config,
            #[cfg(feature = "http")]
            credentials: Some(credentials),
            #[cfg(feature = "http")]
            http: build_http_client()?,
            #[cfg(feature = "http")]
            token_cache: Arc::default(),
        })
    }

    /// Return the configuration used by this client.
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Return application credentials when configured.
    pub fn credentials(&self) -> Option<&AppCredentials> {
        #[cfg(feature = "http")]
        {
            self.credentials.as_ref()
        }
        #[cfg(not(feature = "http"))]
        {
            None
        }
    }

    /// Fetch and cache a tenant access token for the configured application.
    #[cfg(feature = "http")]
    pub async fn tenant_access_token(&self) -> Result<String> {
        let credentials = self
            .credentials
            .as_ref()
            .ok_or_else(|| Error::Config("app credentials are required".into()))?;
        let mut cache = self.token_cache.lock().await;
        if let Some(token) = cache.as_ref() {
            if std::time::Instant::now() < token.refresh_at {
                return Ok(token.token.clone());
            }
        }

        let response = self
            .http
            .post(self.config.api_url(TENANT_TOKEN_PATH))
            .json(&TenantTokenRequest {
                app_id: credentials.app_id.clone(),
                app_secret: credentials.app_secret.clone(),
            })
            .send()
            .await?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("X-Tt-Logid")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.bytes().await?;
        let token_response: TenantTokenResponse = serde_json::from_slice(&body)?;
        if !status.is_success() || token_response.code != 0 {
            return Err(Error::Api {
                code: token_response.code,
                msg: token_response.msg,
                request_id,
            });
        }

        let token = token_response
            .tenant_access_token
            .filter(|token| !token.is_empty())
            .ok_or_else(|| Error::InvalidResponse("tenant token is missing".into()))?;
        let expire_seconds = token_response
            .expire
            .filter(|expire| *expire > TOKEN_REFRESH_MARGIN.as_secs())
            .unwrap_or(TOKEN_REFRESH_MARGIN.as_secs());
        let expire = Duration::from_secs(expire_seconds);
        *cache = Some(CachedTenantToken {
            token: token.clone(),
            refresh_at: std::time::Instant::now() + expire - TOKEN_REFRESH_MARGIN,
        });
        Ok(token)
    }

    #[cfg(feature = "http")]
    pub(crate) async fn request_json<T, B>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: Option<Vec<(&str, String)>>,
        body: Option<&B>,
    ) -> Result<T>
    where
        B: serde::Serialize,
        T: DeserializeOwned,
    {
        let token = self.tenant_access_token().await?;
        let mut request = self
            .http
            .request(method, self.config.api_url(path))
            .bearer_auth(token)
            .header("Content-Type", "application/json");
        if let Some(query) = query {
            request = request.query(&query);
        }
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request.send().await?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("X-Tt-Logid")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        let envelope: ApiResponse<T> = serde_json::from_slice(&bytes)?;
        if !status.is_success() && envelope.code == 0 {
            return Err(Error::InvalidResponse(format!(
                "HTTP {status} without a platform error code"
            )));
        }
        if envelope.code != 0 {
            return Err(Error::Api {
                code: envelope.code,
                msg: envelope.msg,
                request_id,
            });
        }

        envelope
            .data
            .ok_or_else(|| Error::InvalidResponse("response data is missing".into()))
    }

    #[cfg(feature = "http")]
    pub(crate) async fn request_json_with_token<T, B>(
        &self,
        access_token: impl AsRef<str>,
        method: reqwest::Method,
        path: &str,
        query: Option<Vec<(&str, String)>>,
        body: Option<&B>,
    ) -> Result<T>
    where
        B: serde::Serialize,
        T: DeserializeOwned,
    {
        let mut request = self
            .http
            .request(method, self.config.api_url(path))
            .bearer_auth(access_token.as_ref())
            .header("Content-Type", "application/json");
        if let Some(query) = query {
            request = request.query(&query);
        }
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request.send().await?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("X-Tt-Logid")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        let envelope: ApiResponse<T> = serde_json::from_slice(&bytes)?;
        if !status.is_success() && envelope.code == 0 {
            return Err(Error::InvalidResponse(format!(
                "HTTP {status} without a platform error code"
            )));
        }
        if envelope.code != 0 {
            return Err(Error::Api {
                code: envelope.code,
                msg: envelope.msg,
                request_id,
            });
        }

        envelope
            .data
            .ok_or_else(|| Error::InvalidResponse("response data is missing".into()))
    }

    #[cfg(feature = "http")]
    pub(crate) async fn request_empty<B>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: Option<Vec<(&str, String)>>,
        body: Option<&B>,
    ) -> Result<()>
    where
        B: serde::Serialize,
    {
        let token = self.tenant_access_token().await?;
        let mut request = self
            .http
            .request(method, self.config.api_url(path))
            .bearer_auth(token)
            .header("Content-Type", "application/json");
        if let Some(query) = query {
            request = request.query(&query);
        }
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request.send().await?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("X-Tt-Logid")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        // CardKit update endpoints return `data: {}` on success. This helper
        // discards success data, so accept any JSON value instead of unit.
        let envelope: ApiResponse<serde_json::Value> = serde_json::from_slice(&bytes)?;
        if !status.is_success() && envelope.code == 0 {
            return Err(Error::InvalidResponse(format!(
                "HTTP {status} without a platform error code"
            )));
        }
        if envelope.code != 0 {
            return Err(Error::Api {
                code: envelope.code,
                msg: envelope.msg,
                request_id,
            });
        }
        Ok(())
    }
}

#[cfg(feature = "http")]
#[derive(serde::Deserialize)]
pub(crate) struct ApiResponse<T> {
    pub code: i64,
    pub msg: String,
    pub data: Option<T>,
}

#[cfg(feature = "http")]
fn build_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .build()
        .map_err(|error| Error::Config(format!("failed to create HTTP client: {error}")))
}
