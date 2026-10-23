//! Stable error categories for providers and normalized JSON Lines.

use std::fmt;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    Config,
    Credentials,
    UnsupportedCapability,
    Dns,
    Timeout,
    Transport,
    Http,
    Authentication,
    PermissionDenied,
    QuotaExceeded,
    RateLimited,
    Decode,
    Mapping,
    NoData,
    Api,
    Provider,
}

/// A safe diagnostic. Messages must not contain credentials or reflected server text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LookupError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

impl LookupError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            http_status: None,
        }
    }

    pub fn http(status: u16, message: impl Into<String>) -> Self {
        let code = match status {
            401 => ErrorCode::Authentication,
            402 => ErrorCode::QuotaExceeded,
            403 => ErrorCode::PermissionDenied,
            404 => ErrorCode::NoData,
            429 => ErrorCode::RateLimited,
            _ => ErrorCode::Http,
        };
        Self {
            code,
            message: message.into(),
            http_status: Some(status),
        }
    }

    /// Preserve typed errors through anyhow contexts, with a fallback for third-party providers.
    pub fn from_error(error: &anyhow::Error, fallback: ErrorCode) -> Self {
        if let Some(error) = error.downcast_ref::<Self>() {
            return error.clone();
        }
        if let Some(source) = error.downcast_ref::<ureq::Error>() {
            let mut classified = Self::from_http_client(source, "");
            classified.message = format!("{error:#}");
            return classified;
        }
        Self::new(fallback, format!("{error:#}"))
    }

    /// Classify without retaining a source that could contain a secret request URL.
    pub(crate) fn from_http_client(error: &ureq::Error, message: &str) -> Self {
        let code = match error {
            ureq::Error::StatusCode(status) => return Self::http(*status, message),
            ureq::Error::Timeout(_) => ErrorCode::Timeout,
            ureq::Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                ErrorCode::Timeout
            }
            ureq::Error::HostNotFound => ErrorCode::Dns,
            ureq::Error::Json(_) | ureq::Error::BodyExceedsLimit(_) => ErrorCode::Decode,
            _ => ErrorCode::Transport,
        };
        let message = match code {
            ErrorCode::Timeout => format!("{message}: timed out"),
            ErrorCode::Dns => format!("{message}: host not found"),
            _ => message.to_string(),
        };
        Self::new(code, message)
    }
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LookupError {}
