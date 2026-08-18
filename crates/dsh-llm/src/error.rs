//! LLM error type, credential-key checks, and stable failure codes.

use dsh_session::LlmFailure;

/// Canonical provider-neutral code for a supplied credential that cannot be used.
pub const INVALID_CREDENTIAL_CODE: &str = "INVALID_CREDENTIAL";
/// Canonical provider-neutral code for a completed response with no content blocks.
pub const EMPTY_RESPONSE_CODE: &str = "EMPTY_RESPONSE";
/// Canonical provider-neutral code for a request cancelled by the caller.
pub const ABORTED_CODE: &str = "ABORTED";
/// Canonical provider-neutral code for a request that exceeded the model context window.
pub const CONTEXT_WINDOW_EXCEEDED_CODE: &str = "CONTEXT_WINDOW_EXCEEDED";

/// Why a supplied API key cannot be used as an HTTP header value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiKeyRejection {
    /// The trimmed key is empty.
    Empty,
    /// The trimmed key contains a character outside printable ASCII without space.
    IllegalCharacters,
}

/// Typed LLM failure retained beside a terminal finish chunk.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct LlmError {
    /// Human-readable failure summary.
    pub message: String,
    /// Stable machine-routing code.
    pub code: String,
    /// HTTP status when available.
    pub status: Option<u16>,
    /// Provider-requested delay in milliseconds.
    pub provider_retry_after_ms: Option<u64>,
    /// Provider request id when available.
    pub request_id: Option<String>,
}

impl LlmError {
    /// Construct a failure with `message` and `code` and no provider facts.
    #[must_use]
    pub fn new(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: code.into(),
            status: None,
            provider_retry_after_ms: None,
            request_id: None,
        }
    }

    /// Attach an HTTP status to this failure.
    #[must_use]
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// Serializable facts for a terminal [`dsh_session::FinishReason`].
    #[must_use]
    pub fn failure(&self) -> LlmFailure {
        LlmFailure {
            message: self.message.clone(),
            code: self.code.clone(),
            status: self.status,
            provider_retry_after_ms: self.provider_retry_after_ms,
            request_id: self.request_id.clone(),
        }
    }
}

/// Trim `raw` and accept a key that HTTP headers can carry.
///
/// # Errors
///
/// Returns [`ApiKeyRejection::Empty`] when the trimmed key is empty, or
/// [`ApiKeyRejection::IllegalCharacters`] when it is not printable ASCII
/// without space (`[\x21-\x7E]+`).
pub fn normalize_api_key(raw: &str) -> Result<String, ApiKeyRejection> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(ApiKeyRejection::Empty);
    }
    if !value.bytes().all(|byte| (0x21..=0x7E).contains(&byte)) {
        return Err(ApiKeyRejection::IllegalCharacters);
    }
    Ok(value.to_string())
}

/// Accept one supplied credential, or refuse it as unusable without echoing the secret.
///
/// # Errors
///
/// Returns [`LlmError`] with [`INVALID_CREDENTIAL_CODE`] when [`normalize_api_key`]
/// rejects `raw`. The message names `pkg` and `ref_name`, never the key bytes.
pub fn assert_usable_api_key(raw: &str, pkg: &str, ref_name: &str) -> Result<String, LlmError> {
    match normalize_api_key(raw) {
        Ok(value) => Ok(value),
        Err(ApiKeyRejection::Empty) => Err(LlmError::new(
            format!(
                "{pkg}: the API key resolved from {ref_name} is blank; set {ref_name} to the raw key (the web Models page writes it) or export it in the launching environment"
            ),
            INVALID_CREDENTIAL_CODE,
        )),
        Err(ApiKeyRejection::IllegalCharacters) => Err(LlmError::new(
            format!(
                "{pkg}: the API key resolved from {ref_name} contains characters no HTTP header can carry; set {ref_name} to the raw key alone (the web Models page writes it)"
            ),
            INVALID_CREDENTIAL_CODE,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApiKeyRejection, INVALID_CREDENTIAL_CODE, assert_usable_api_key, normalize_api_key,
    };

    #[test]
    fn trims_and_accepts_printable_key() {
        assert_eq!(normalize_api_key("  sk-live  ").unwrap(), "sk-live");
    }

    #[test]
    fn blank_and_illegal_keys_are_diagnosed_without_echoing_the_secret() {
        assert!(matches!(
            normalize_api_key("   "),
            Err(ApiKeyRejection::Empty)
        ));
        assert!(matches!(
            normalize_api_key("sk live"),
            Err(ApiKeyRejection::IllegalCharacters)
        ));
        let empty =
            assert_usable_api_key("  ", "dsh-llm-deepseek", "DEEPSEEK_API_KEY").unwrap_err();
        assert_eq!(empty.code, INVALID_CREDENTIAL_CODE);
        assert!(empty.message.contains("DEEPSEEK_API_KEY"));
        assert!(!empty.message.contains("sk-"));
        let illegal =
            assert_usable_api_key("sk\nkey", "dsh-llm-deepseek", "DEEPSEEK_API_KEY").unwrap_err();
        assert_eq!(illegal.code, INVALID_CREDENTIAL_CODE);
        assert!(
            illegal
                .message
                .contains("characters no HTTP header can carry")
        );
    }
}
