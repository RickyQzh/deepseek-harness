//! Closed [`RpcErrorCode`] set and the `{ code, message, details }` wire error.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Closed error-code union. Unknown kebab-case codes fail decode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RpcErrorCode {
    /// Wire code `bad-request`.
    BadRequest,
    /// Wire code `cancelled`.
    Cancelled,
    /// Wire code `session-not-found`.
    SessionNotFound,
    /// Wire code `model-unavailable`.
    ModelUnavailable,
    /// Wire code `session-conflict`.
    SessionConflict,
    /// Wire code `invalid-time-zone`.
    InvalidTimeZone,
    /// Wire code `workspace-attach-failed`.
    WorkspaceAttachFailed,
    /// Wire code `workspace-not-found`.
    WorkspaceNotFound,
    /// Wire code `workspace-invalid-path`.
    WorkspaceInvalidPath,
    /// Wire code `workspace-name-conflict`.
    WorkspaceNameConflict,
    /// Wire code `workspace-move-invalid`.
    WorkspaceMoveInvalid,
    /// Wire code `directory-unreadable`.
    DirectoryUnreadable,
    /// Wire code `directory-exists`.
    DirectoryExists,
    /// Wire code `directory-create-failed`.
    DirectoryCreateFailed,
    /// Wire code `directory-picker-unavailable`.
    DirectoryPickerUnavailable,
    /// Wire code `agent-preset-read-only`.
    AgentPresetReadOnly,
    /// Wire code `agent-preset-locked`.
    AgentPresetLocked,
    /// Wire code `agent-preset-conflict`.
    AgentPresetConflict,
    /// Wire code `agent-preset-not-found`.
    AgentPresetNotFound,
    /// Wire code `agent-preset-invalid`.
    AgentPresetInvalid,
    /// Wire code `agent-busy`.
    AgentBusy,
    /// Wire code `attachment-error`.
    AttachmentError,
    /// Wire code `queue-item-not-found`.
    QueueItemNotFound,
    /// Wire code `steer-unavailable`.
    SteerUnavailable,
    /// Wire code `command-error`.
    CommandError,
    /// Wire code `unknown-command`.
    UnknownCommand,
    /// Wire code `settings-rejected`.
    SettingsRejected,
    /// Wire code `settings-not-exposed`.
    SettingsNotExposed,
    /// Wire code `settings-conflict`.
    SettingsConflict,
    /// Wire code `credential-rejected`.
    CredentialRejected,
    /// Wire code `model-discovery-failed`.
    ModelDiscoveryFailed,
    /// Wire code `title-invalid`.
    TitleInvalid,
    /// Wire code `fork-unavailable`.
    ForkUnavailable,
    /// Wire code `subagent-parent-unavailable`.
    SubagentParentUnavailable,
    /// Wire code `subagent-not-found`.
    SubagentNotFound,
    /// Wire code `subagent-catalog-diagnostic`.
    SubagentCatalogDiagnostic,
    /// Wire code `subagent-not-resumable`.
    SubagentNotResumable,
    /// Wire code `subagent-unauthorized`.
    SubagentUnauthorized,
    /// Wire code `subagent-delivery-unavailable`.
    SubagentDeliveryUnavailable,
    /// Wire code `internal`.
    Internal,
}

/// Wire error object `{ code, message, details }`. `details` is required (`{}` is valid).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    code: RpcErrorCode,
    message: String,
    details: Value,
}

impl RpcError {
    /// Fold a transport or unexpected failure into `internal` with empty details.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::with_code(
            RpcErrorCode::Internal,
            message,
            Value::Object(serde_json::Map::new()),
        )
    }

    /// Build an error with an explicit code and details object.
    #[must_use]
    pub fn with_code(code: RpcErrorCode, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details,
        }
    }

    /// Closed error code.
    #[must_use]
    pub fn code(&self) -> RpcErrorCode {
        self.code
    }

    /// Human-readable failure text.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Required details JSON; empty object when the code carries no fields.
    #[must_use]
    pub fn details(&self) -> &Value {
        &self.details
    }
}
