//! Error type hierarchy mirroring the Go SDK's `apierror` package.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Top-level error. Wraps transport, decode, and API-returned errors so
/// callers have a single type to match on.
#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP transport: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("JSON decode: {0}")]
    Decode(#[from] serde_json::Error),

    #[error("API error ({status}): {message}{request_id}", request_id = .request_id.as_ref().map(|r| format!(" (Request-ID: {r})")).unwrap_or_default())]
    Api {
        status: reqwest::StatusCode,
        message: String,
        request_id: Option<String>,
        body: Option<serde_json::Value>,
    },

    #[error("Authentication failed: {0}")]
    Authentication(String),

    #[error("Rate limited: retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("Stream parse error: {0}")]
    Stream(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Transport(e) => e.is_timeout() || e.is_connect() || e.is_request(),
            Error::Api { status, .. } => {
                let code = status.as_u16();
                matches!(code, 408 | 409 | 425 | 429) || code >= 500
            }
            Error::RateLimited { .. } => true,
            _ => false,
        }
    }
}
