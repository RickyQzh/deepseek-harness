//! In-process mutation fence for [`crate::LocalFileSystem`].

use dsh_sandbox::{SandboxExecutionPolicy, SandboxMode, writable_roots};

use crate::containment::is_path_under;
use crate::{FsError, FsErrorCode, FsTarget, LocalFileSystem};

/// Default file-effect policy installed on a sandboxed [`LocalFileSystem`].
///
/// The fence is in-process containment over a model-controlled path, not a
/// kernel boundary.
#[derive(Clone, Debug)]
pub struct SandboxFence {
    /// Policy used when a mutation omits `sandbox_policy`.
    pub default_policy: SandboxExecutionPolicy,
}

/// Enforce `sandbox_policy` against `target` and return the target the mutation must use.
///
/// [`SandboxMode::DangerFullAccess`] returns `target` unchanged.
/// [`SandboxMode::ReadOnly`] is [`FsErrorCode::SandboxDenied`].
/// [`SandboxMode::WorkspaceWrite`] re-resolves `target.display_path` and requires
/// [`is_path_under`] some [`writable_roots`] entry, then returns that fresh target.
/// [`None`] returns `target` unchanged.
///
/// # Errors
///
/// [`FsErrorCode::SandboxDenied`] for a refused mutation. Resolve failures from a
/// workspace-write re-check are forwarded.
pub async fn checked_target(
    fs: &LocalFileSystem,
    target: &FsTarget,
    sandbox_policy: Option<&SandboxExecutionPolicy>,
) -> Result<FsTarget, FsError> {
    let Some(policy) = sandbox_policy else {
        return Ok(target.clone());
    };
    match policy.mode {
        SandboxMode::DangerFullAccess => Ok(target.clone()),
        SandboxMode::ReadOnly => Err(FsError::new(
            format!(
                "cannot write \"{}\": file access denied under read-only mode",
                target.display_path
            ),
            FsErrorCode::SandboxDenied,
        )),
        SandboxMode::WorkspaceWrite => {
            let fresh = fs.resolve(&target.display_path, None, None).await?;
            for root in writable_roots(policy) {
                if is_path_under(fresh.target_key.as_str(), &root).await {
                    return Ok(fresh);
                }
            }
            Err(FsError::new(
                format!(
                    "cannot write \"{}\": file access denied under workspace-write mode",
                    target.display_path
                ),
                FsErrorCode::SandboxDenied,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::checked_target;
    use crate::{FsEditRequest, FsErrorCode, LocalFileSystem};
    use dsh_sandbox::{SandboxExecutionPolicy, SandboxMode};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    struct TempRoot(PathBuf);

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn unique_dir(parent: &Path) -> PathBuf {
        let dir = parent.join(format!(
            "dsh-fs-fence-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn home_base() -> TempRoot {
        let parent = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| PathBuf::from("/var/tmp"));
        TempRoot(unique_dir(&parent))
    }

    fn policy(mode: SandboxMode, workspace: &Path) -> SandboxExecutionPolicy {
        SandboxExecutionPolicy {
            mode,
            workspace_root: workspace.to_string_lossy().into_owned(),
            session_id: None,
        }
    }

    #[tokio::test]
    async fn read_only_denies_write_and_does_not_create_the_file() {
        let root = home_base();
        let ws = root.0.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let fs = LocalFileSystem::sandboxed(&ws, policy(SandboxMode::ReadOnly, &ws));
        assert_eq!(fs.sandbox_mode(), Some(SandboxMode::ReadOnly));
        let target = fs.resolve("denied.txt", None, None).await.unwrap();
        let err = fs
            .write_text(&target, "x", None, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::SandboxDenied);
        assert_eq!(
            err.message,
            format!(
                "cannot write \"{}\": file access denied under read-only mode",
                target.display_path
            )
        );
        assert!(!ws.join("denied.txt").exists());
    }

    #[tokio::test]
    async fn read_only_allows_reads() {
        let root = home_base();
        let ws = root.0.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("readable.txt"), "hello").unwrap();
        let fs = LocalFileSystem::sandboxed(&ws, policy(SandboxMode::ReadOnly, &ws));
        let target = fs.resolve("readable.txt", None, None).await.unwrap();
        assert_eq!(fs.read_text(&target, None).await.unwrap(), "hello");
    }

    #[tokio::test]
    async fn read_only_denies_edit_and_leaves_content() {
        let root = home_base();
        let ws = root.0.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("file.txt"), "original").unwrap();
        let fs = LocalFileSystem::sandboxed(&ws, policy(SandboxMode::ReadOnly, &ws));
        let target = fs.resolve("file.txt", None, None).await.unwrap();
        let err = fs
            .edit_text(
                &target,
                &FsEditRequest {
                    old_string: "original".into(),
                    new_string: "changed".into(),
                    replace_all: false,
                },
                None,
                None,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::SandboxDenied);
        assert_eq!(
            std::fs::read_to_string(ws.join("file.txt")).unwrap(),
            "original"
        );
    }

    #[tokio::test]
    async fn workspace_write_allows_a_file_under_the_workspace() {
        let root = home_base();
        let ws = root.0.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let fs = LocalFileSystem::sandboxed(&ws, policy(SandboxMode::WorkspaceWrite, &ws));
        assert_eq!(fs.sandbox_mode(), Some(SandboxMode::WorkspaceWrite));
        let target = fs.resolve("nested/ok.txt", None, None).await.unwrap();
        fs.write_text(&target, "inside", None, None, None)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(ws.join("nested/ok.txt")).unwrap(),
            "inside"
        );
    }

    #[tokio::test]
    async fn workspace_write_denies_a_file_outside_writable_roots() {
        let root = home_base();
        let ws = root.0.join("ws");
        let outside = root.0.join("out");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let fs = LocalFileSystem::sandboxed(&ws, policy(SandboxMode::WorkspaceWrite, &ws));
        let target = fs
            .resolve(outside.join("escape.txt").to_str().unwrap(), None, None)
            .await
            .unwrap();
        let err = fs
            .write_text(&target, "x", None, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.code, FsErrorCode::SandboxDenied);
        assert_eq!(
            err.message,
            format!(
                "cannot write \"{}\": file access denied under workspace-write mode",
                target.display_path
            )
        );
        assert!(!outside.join("escape.txt").exists());

        if Path::new("/etc/hostname").exists() {
            let host = fs.resolve("/etc/hostname", None, None).await.unwrap();
            let err = fs
                .write_text(&host, "x", None, None, None)
                .await
                .unwrap_err();
            assert_eq!(err.code, FsErrorCode::SandboxDenied);
        }
    }

    #[tokio::test]
    async fn danger_full_access_allows_a_write_outside_the_workspace() {
        let root = home_base();
        let ws = root.0.join("ws");
        let outside = root.0.join("out");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let fs = LocalFileSystem::sandboxed(&ws, policy(SandboxMode::DangerFullAccess, &ws));
        assert_eq!(fs.sandbox_mode(), Some(SandboxMode::DangerFullAccess));
        let target = fs
            .resolve(outside.join("free.txt").to_str().unwrap(), None, None)
            .await
            .unwrap();
        fs.write_text(&target, "free", None, None, None)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(outside.join("free.txt")).unwrap(),
            "free"
        );
    }

    #[tokio::test]
    async fn unfenced_new_ignores_sandbox_policy() {
        let root = home_base();
        let ws = root.0.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let fs = LocalFileSystem::new(&ws);
        assert!(fs.sandbox_mode().is_none());
        let target = fs.resolve("a.txt", None, None).await.unwrap();
        let denied = policy(SandboxMode::ReadOnly, &ws);
        fs.write_text(&target, "ok", None, None, Some(&denied))
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "ok");
    }

    #[tokio::test]
    async fn per_call_policy_overrides_fence_default() {
        let root = home_base();
        let ws = root.0.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let fs = LocalFileSystem::sandboxed(&ws, policy(SandboxMode::ReadOnly, &ws));
        let target = fs.resolve("escalated.txt", None, None).await.unwrap();
        let wider = policy(SandboxMode::WorkspaceWrite, &ws);
        fs.write_text(&target, "ok", None, None, Some(&wider))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(ws.join("escalated.txt")).unwrap(),
            "ok"
        );
    }

    #[tokio::test]
    async fn checked_target_danger_returns_original() {
        let root = home_base();
        let ws = root.0.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let fs = LocalFileSystem::sandboxed(&ws, policy(SandboxMode::DangerFullAccess, &ws));
        let target = fs.resolve("a.txt", None, None).await.unwrap();
        let policy = policy(SandboxMode::DangerFullAccess, &ws);
        let out = checked_target(&fs, &target, Some(&policy)).await.unwrap();
        assert_eq!(out, target);
    }
}
