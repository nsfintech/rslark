//! Authentication credentials and tenant token handling.

/// Credentials for a self-built Feishu or Lark application.
#[derive(Clone, Eq, PartialEq)]
pub struct AppCredentials {
    /// Application ID, usually beginning with `cli_`.
    pub app_id: String,
    /// Application secret.
    pub app_secret: String,
}

impl std::fmt::Debug for AppCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppCredentials")
            .field("app_id", &self.app_id)
            .field("app_secret", &"[redacted]")
            .finish()
    }
}

impl AppCredentials {
    /// Create credentials for a self-built application.
    pub fn new(app_id: impl Into<String>, app_secret: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            app_secret: app_secret.into(),
        }
    }
}

#[cfg(feature = "http")]
#[derive(serde::Serialize)]
pub(crate) struct TenantTokenRequest {
    pub app_id: String,
    pub app_secret: String,
}

#[cfg(feature = "http")]
#[derive(serde::Deserialize)]
pub(crate) struct TenantTokenResponse {
    pub code: i64,
    pub msg: String,
    pub tenant_access_token: Option<String>,
    pub expire: Option<u64>,
}

/// A cached tenant access token and the time at which it should be refreshed.
#[cfg(feature = "http")]
#[derive(Debug)]
pub(crate) struct CachedTenantToken {
    pub token: String,
    pub refresh_at: std::time::Instant,
}
