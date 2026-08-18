//! Canonical path identity and writable-root derivation.

use std::collections::BTreeSet;

use crate::types::{SandboxExecutionPolicy, SandboxMode};

/// Resolve `path` with [`std::fs::canonicalize`]. Returns the input spelling when resolution fails.
#[must_use]
pub fn canonical_path(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(resolved) => resolved.to_string_lossy().into_owned(),
        Err(_) => path.to_string(),
    }
}

/// Canonical writable roots for `policy`. Empty for `read-only` and `danger-full-access`.
///
/// Under `workspace-write`, the set is the policy workspace root, `/tmp`, and the platform
/// temp directory, each passed through [`canonical_path`] and deduplicated.
#[must_use]
pub fn writable_roots(policy: &SandboxExecutionPolicy) -> Vec<String> {
    match policy.mode {
        SandboxMode::WorkspaceWrite => {
            let mut roots = BTreeSet::new();
            roots.insert(canonical_path(&policy.workspace_root));
            roots.insert(canonical_path("/tmp"));
            roots.insert(canonical_path(&std::env::temp_dir().to_string_lossy()));
            roots.into_iter().collect()
        }
        SandboxMode::ReadOnly | SandboxMode::DangerFullAccess => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_path, writable_roots};
    use crate::{SandboxExecutionPolicy, SandboxMode};

    #[test]
    fn read_only_writable_roots_are_empty() {
        let policy = SandboxExecutionPolicy {
            mode: SandboxMode::ReadOnly,
            workspace_root: "/ws".into(),
            session_id: None,
        };
        assert!(writable_roots(&policy).is_empty());
    }

    #[test]
    fn workspace_write_includes_workspace_tmp_and_platform_temp() {
        let ws = std::env::temp_dir().join("dsh-ws-roots");
        std::fs::create_dir_all(&ws).unwrap();
        let policy = SandboxExecutionPolicy {
            mode: SandboxMode::WorkspaceWrite,
            workspace_root: ws.to_string_lossy().into_owned(),
            session_id: None,
        };
        let roots = writable_roots(&policy);
        assert!(
            roots
                .iter()
                .any(|r| r == &canonical_path(&policy.workspace_root))
        );
        assert!(roots.iter().any(|r| r == &canonical_path("/tmp")));
        assert!(
            roots
                .iter()
                .any(|r| r == &canonical_path(&std::env::temp_dir().to_string_lossy()))
        );
    }

    #[test]
    fn missing_path_canonicalizes_to_the_spelling() {
        assert_eq!(
            canonical_path("/definitely-missing-dsh-phase4"),
            "/definitely-missing-dsh-phase4"
        );
    }

    #[test]
    fn danger_full_access_writable_roots_are_empty() {
        let policy = SandboxExecutionPolicy {
            mode: SandboxMode::DangerFullAccess,
            workspace_root: "/ws".into(),
            session_id: None,
        };
        assert!(writable_roots(&policy).is_empty());
    }
}
