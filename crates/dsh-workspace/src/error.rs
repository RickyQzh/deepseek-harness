//! Workspace registry failures and kebab-case wire codes.

use dsh_rpc::RpcErrorCode;

/// Durable registry failure. `code` is the kebab-case wire string.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WorkspaceError {
    /// Create path is missing, not a directory, or cannot be canonicalized.
    #[error("cannot create a workspace at a missing or non-directory path")]
    InvalidPath,
    /// Rename, delete, reorder, or attach named an unknown workspace id.
    #[error("workspace not found")]
    NotFound,
    /// Rename title equals another workspace's title.
    #[error("workspace title conflicts with another workspace")]
    NameConflict,
    /// Rename title is empty after trim.
    #[error("workspace title is empty after trim")]
    TitleInvalid,
    /// `insert_session_before` named a session or anchor not on that workspace.
    #[error("session is not accounted on that workspace")]
    MoveInvalid,
    /// Persist JSON is not an object of the expected fields, or order/map diverge.
    #[error("workspace persist file is corrupt: {0}")]
    Corrupt(String),
    /// Filesystem failure while creating the parent directory or writing JSON.
    #[error("workspace persist failed: {0}")]
    Io(String),
    /// Plugin load has no config `path` and neither env value is set.
    #[error("DSH_HOME or DSH_SESSION_ROOT must be set for the workspace registry")]
    MissingPersistPath,
}

impl WorkspaceError {
    /// Kebab-case wire code for this failure.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPath => "workspace-invalid-path",
            Self::NotFound => "workspace-not-found",
            Self::NameConflict => "workspace-name-conflict",
            Self::TitleInvalid => "title-invalid",
            Self::MoveInvalid => "workspace-move-invalid",
            Self::Corrupt(_) | Self::Io(_) | Self::MissingPersistPath => "internal",
        }
    }

    /// Closed RPC error code for this failure.
    #[must_use]
    pub fn rpc_code(&self) -> RpcErrorCode {
        match self {
            Self::InvalidPath => RpcErrorCode::WorkspaceInvalidPath,
            Self::NotFound => RpcErrorCode::WorkspaceNotFound,
            Self::NameConflict => RpcErrorCode::WorkspaceNameConflict,
            Self::TitleInvalid => RpcErrorCode::TitleInvalid,
            Self::MoveInvalid => RpcErrorCode::WorkspaceMoveInvalid,
            Self::Corrupt(_) | Self::Io(_) | Self::MissingPersistPath => RpcErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceError;
    use dsh_rpc::RpcErrorCode;

    #[test]
    fn invalid_path_maps_to_workspace_invalid_path() {
        let err = WorkspaceError::InvalidPath;
        assert_eq!(err.code(), "workspace-invalid-path");
        assert_eq!(err.rpc_code(), RpcErrorCode::WorkspaceInvalidPath);
    }

    #[test]
    fn persist_failures_map_to_internal() {
        let err = WorkspaceError::Corrupt("bad json".into());
        assert_eq!(err.code(), "internal");
        assert_eq!(err.rpc_code(), RpcErrorCode::Internal);
    }
}
