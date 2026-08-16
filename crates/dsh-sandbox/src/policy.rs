//! Per-call sandbox policy resolution from a constructor default.

use dsh_session::SessionId;

use crate::roots::canonical_path;
use crate::types::{SandboxExecutionPolicy, SandboxMode};

/// Deployment default mode and fallback workspace root for one resolver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxPolicyResolver {
    /// Mode used when no per-call override exists. This crate does not fold `sandbox/mode` events.
    pub default_mode: SandboxMode,
    /// Fallback root for calls without a session cwd, stored as constructed.
    pub workspace_root: String,
}

impl SandboxPolicyResolver {
    /// Store `default_mode` and `workspace_root` as the resolve fallbacks.
    #[must_use]
    pub fn new(default_mode: SandboxMode, workspace_root: String) -> Self {
        Self {
            default_mode,
            workspace_root,
        }
    }

    /// Resolve per-call mode and canonical workspace root.
    ///
    /// `workspace_root` is [`crate::canonical_path`] of `session_cwd` when present, otherwise of
    /// the constructor root. `mode` is always [`Self::default_mode`].
    #[must_use]
    pub fn resolve(
        &self,
        session_id: Option<SessionId>,
        session_cwd: Option<&str>,
    ) -> SandboxExecutionPolicy {
        let workspace_root = match session_cwd {
            Some(cwd) => canonical_path(cwd),
            None => canonical_path(&self.workspace_root),
        };
        SandboxExecutionPolicy {
            mode: self.default_mode,
            workspace_root,
            session_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SandboxPolicyResolver;
    use crate::{SandboxMode, canonical_path};
    use dsh_session::SessionId;

    fn existing_dir(name: &str) -> String {
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn resolver_without_session_cwd_uses_constructor_root() {
        let root = existing_dir("dsh-sandbox-policy-fallback");
        let resolver = SandboxPolicyResolver::new(SandboxMode::ReadOnly, root.clone());
        let policy = resolver.resolve(None, None);
        assert_eq!(policy.mode, SandboxMode::ReadOnly);
        assert_eq!(policy.workspace_root, canonical_path(&root));
        assert!(policy.session_id.is_none());
    }

    #[test]
    fn resolver_with_session_cwd_uses_that_cwd() {
        let fallback = existing_dir("dsh-sandbox-policy-unused-fallback");
        let cwd = existing_dir("dsh-sandbox-policy-session-cwd");
        let resolver = SandboxPolicyResolver::new(SandboxMode::WorkspaceWrite, fallback);
        let session_id = SessionId::new("sess-sandbox-policy");
        let policy = resolver.resolve(Some(session_id.clone()), Some(&cwd));
        assert_eq!(policy.mode, SandboxMode::WorkspaceWrite);
        assert_eq!(policy.workspace_root, canonical_path(&cwd));
        assert_eq!(policy.session_id, Some(session_id));
    }

    #[test]
    fn resolver_default_mode_is_the_constructed_value() {
        let root = existing_dir("dsh-sandbox-policy-mode");
        let danger = SandboxPolicyResolver::new(SandboxMode::DangerFullAccess, root.clone())
            .resolve(None, None);
        assert_eq!(danger.mode, SandboxMode::DangerFullAccess);
        let write =
            SandboxPolicyResolver::new(SandboxMode::WorkspaceWrite, root).resolve(None, None);
        assert_eq!(write.mode, SandboxMode::WorkspaceWrite);
    }
}
