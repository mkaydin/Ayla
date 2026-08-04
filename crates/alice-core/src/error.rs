use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Stable error envelope for Tauri adapters and the future Electron sidecar.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackendError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl BackendError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            details: None,
        }
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

impl From<String> for BackendError {
    fn from(message: String) -> Self {
        Self::new("backend_error", message)
    }
}

impl From<&str> for BackendError {
    fn from(message: &str) -> Self {
        Self::new("backend_error", message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_envelope_is_stable_and_serializable() {
        let error = BackendError::new("db_offline", "Database unavailable").retryable(true);
        let encoded = serde_json::to_value(&error).unwrap();
        assert_eq!(encoded["code"], "db_offline");
        assert_eq!(encoded["message"], "Database unavailable");
        assert_eq!(encoded["retryable"], true);
        assert!(encoded.get("details").is_none());
    }
}
