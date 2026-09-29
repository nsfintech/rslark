//! Client configuration and top-level SDK client.

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
}

/// Top-level entry point for SDK functionality.
#[derive(Clone, Debug, Default)]
pub struct Client {
    config: ClientConfig,
}

impl Client {
    /// Create a client with the default Feishu configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a client with explicit configuration.
    pub fn with_config(config: ClientConfig) -> Self {
        Self { config }
    }

    /// Return the configuration used by this client.
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }
}
