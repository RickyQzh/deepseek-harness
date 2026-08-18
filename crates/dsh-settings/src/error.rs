//! Settings failures and kebab-case wire codes.

use dsh_rpc::RpcErrorCode;
use serde_json::{Value, json};

/// Settings namespace failure. `code` is the kebab-case wire string.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SettingsError {
    /// Caller named a namespace this crate does not expose.
    #[error("settings namespace \"{0}\" is not exposed")]
    NotExposed(String),
    /// `expected` does not match the stored revision.
    #[error(
        "settings namespace \"{ns}\" changed since it was read (expected revision {expected}, now {actual})"
    )]
    Conflict {
        /// Namespace key.
        ns: String,
        /// Revision the caller sent.
        expected: u64,
        /// Revision now stored.
        actual: u64,
    },
    /// Patch, section, or mutate root was not a JSON object.
    #[error("{0}")]
    Rejected(String),
    /// Persist JSON is not `{ revision, value }` with an object `value`.
    #[error("settings persist file is corrupt: {0}")]
    Corrupt(String),
    /// Filesystem failure while creating the directory or writing JSON.
    #[error("settings persist failed: {0}")]
    Io(String),
}

impl SettingsError {
    /// Kebab-case wire code for this failure.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotExposed(_) => "settings-not-exposed",
            Self::Conflict { .. } => "settings-conflict",
            Self::Rejected(_) => "settings-rejected",
            Self::Corrupt(_) | Self::Io(_) => "internal",
        }
    }

    /// Closed RPC error code for this failure.
    #[must_use]
    pub fn rpc_code(&self) -> RpcErrorCode {
        match self {
            Self::NotExposed(_) => RpcErrorCode::SettingsNotExposed,
            Self::Conflict { .. } => RpcErrorCode::SettingsConflict,
            Self::Rejected(_) => RpcErrorCode::SettingsRejected,
            Self::Corrupt(_) | Self::Io(_) => RpcErrorCode::Internal,
        }
    }

    /// JSON details object for the wire error (`{}` when the code carries no fields).
    #[must_use]
    pub fn details(&self) -> Value {
        match self {
            Self::NotExposed(ns) => json!({ "ns": ns }),
            Self::Conflict {
                ns,
                expected,
                actual,
            } => json!({ "ns": ns, "expected": expected, "actual": actual }),
            Self::Rejected(_) | Self::Corrupt(_) | Self::Io(_) => json!({}),
        }
    }

    pub(crate) fn not_exposed(ns: impl Into<String>) -> Self {
        Self::NotExposed(ns.into())
    }

    pub(crate) fn conflict(ns: impl Into<String>, expected: u64, actual: u64) -> Self {
        Self::Conflict {
            ns: ns.into(),
            expected,
            actual,
        }
    }

    pub(crate) fn rejected(message: impl Into<String>) -> Self {
        Self::Rejected(message.into())
    }

    pub(crate) fn corrupt(message: impl Into<String>) -> Self {
        Self::Corrupt(message.into())
    }

    pub(crate) fn io(message: impl Into<String>) -> Self {
        Self::Io(message.into())
    }
}

#[cfg(test)]
mod tests {
    use super::SettingsError;
    use dsh_rpc::RpcErrorCode;
    use serde_json::json;

    #[test]
    fn not_exposed_maps_to_settings_not_exposed() {
        let err = SettingsError::not_exposed("nope");
        assert_eq!(err.code(), "settings-not-exposed");
        assert_eq!(err.rpc_code(), RpcErrorCode::SettingsNotExposed);
        assert_eq!(err.details(), json!({ "ns": "nope" }));
    }

    #[test]
    fn persist_failures_map_to_internal() {
        let err = SettingsError::corrupt("bad json");
        assert_eq!(err.code(), "internal");
        assert_eq!(err.rpc_code(), RpcErrorCode::Internal);
    }
}
