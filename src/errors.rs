// src/errors.rs

use thiserror::Error;

pub type Result<T, E = MspError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum MspError {
    #[error("Network error on {endpoint}: {source}")]
    Network {
        endpoint: &'static str,
        #[source]
        source:   wreq::Error,
    },

    #[error("Request to {endpoint} timed out after {elapsed_ms}ms")]
    Timeout {
        endpoint:   &'static str,
        elapsed_ms: u64,
    },

    #[error("Rate limited on {endpoint} (retry after {retry_after_secs}s)")]
    RateLimited {
        endpoint:         &'static str,
        retry_after_secs: u64,
    },

    #[error("Invalid username or password for {username}@{region}")]
    InvalidCredentials {
        username: String,
        region:   String,
    },

    #[error("Account banned for {username}@{region}: {reason}")]
    AccountBanned {
        username: String,
        region:   String,
        reason:   String,
    },

    #[error("Authentication failed for {username}@{region}: {reason}")]
    AuthFailed {
        username: String,
        region:   String,
        reason:   String,
    },

    #[error("No active session – call auth().login() first")]
    NoSession,

    #[error("Session expired and automatic refresh failed: {reason}")]
    SessionExpired { reason: String },

    #[error("API error on {endpoint} (HTTP {status}){}: {message}",
        .code.as_deref().map(|c| format!(" [{c}]")).unwrap_or_default())]
    Api {
        endpoint: &'static str,
        status:   u16,
        code:     Option<String>,
        message:  String,
    },

    #[error("GraphQL error on {endpoint}: {message}")]
    GraphQl {
        endpoint: &'static str,
        message:  String,
    },

    #[error("JSON error on {endpoint}: {source}")]
    Deserialize {
        endpoint: &'static str,
        #[source]
        source:   serde_json::Error,
    },

    #[error("Proxy configuration error: {reason}")]
    Proxy { reason: String },

    #[error("Session state error: {reason}")]
    State { reason: String },

    #[error("URL encoding failed: {0}")]
    UrlEncoded(#[from] serde_urlencoded::ser::Error),

    #[error("Base64 decoding failed: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("JWT error: {0}")]
    Jwt(String),
}

impl MspError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network { .. }        => true,
            Self::Timeout { .. }        => true,
            Self::RateLimited { .. }    => true,
            Self::SessionExpired { .. } => true,
            Self::Api { status, .. }    => *status >= 500,
            _ => false,
        }
    }

    #[must_use]
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::RateLimited { retry_after_secs, .. } =>
                Some(std::time::Duration::from_secs(*retry_after_secs)),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn from_wreq(err: wreq::Error, endpoint: &'static str) -> Self {
        if err.is_timeout() {
            Self::Timeout { endpoint, elapsed_ms: 0 }
        } else {
            Self::Network { endpoint, source: err }
        }
    }

    #[must_use]
    pub(crate) fn deserialize(err: serde_json::Error, endpoint: &'static str) -> Self {
        Self::Deserialize { endpoint, source: err }
    }

    #[must_use]
    pub(crate) fn api(endpoint: &'static str, status: u16, message: impl Into<String>) -> Self {
        Self::Api { endpoint, status, code: None, message: message.into() }
    }

    #[allow(dead_code)]
    #[must_use]
    pub(crate) fn api_coded(
        endpoint: &'static str,
        status:   u16,
        code:     impl Into<String>,
        message:  impl Into<String>,
    ) -> Self {
        Self::Api {
            endpoint,
            status,
            code:    Some(code.into()),
            message: message.into(),
        }
    }

    #[must_use]
    pub(crate) fn graphql(endpoint: &'static str, message: impl Into<String>) -> Self {
        Self::GraphQl { endpoint, message: message.into() }
    }

    #[must_use]
    pub(crate) fn proxy(reason: impl Into<String>) -> Self {
        Self::Proxy { reason: reason.into() }
    }

    #[must_use]
    pub(crate) fn state(reason: impl Into<String>) -> Self {
        Self::State { reason: reason.into() }
    }
}