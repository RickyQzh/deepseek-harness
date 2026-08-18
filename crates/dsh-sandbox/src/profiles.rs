//! Platform profile argv builders (arguments before the trailing `--` separator).

use crate::landlock::grant_args;
use crate::roots::writable_roots;
use crate::types::{SandboxExecutionPolicy, SandboxMode, SandboxPolicy};

/// Bubblewrap mount profile for `policy` (read-only tree, plus workspace bind under `workspace-write`).
#[must_use]
pub fn bwrap_profile_args(policy: &SandboxPolicy) -> Vec<String> {
    let mut args = vec![
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--die-with-parent".into(),
    ];
    if policy.mode == SandboxMode::WorkspaceWrite {
        args.extend([
            "--tmpfs".into(),
            "/tmp".into(),
            "--bind".into(),
            policy.workspace_root.clone(),
            policy.workspace_root.clone(),
        ]);
    }
    args
}

/// Landlock launcher grants for `policy` (`/` read-only, `/dev/null` writable, plus `/tmp` and the workspace under `workspace-write`).
#[must_use]
pub fn landlock_profile_args(policy: &SandboxPolicy) -> Vec<String> {
    let mut read_write = vec!["/dev/null"];
    if policy.mode == SandboxMode::WorkspaceWrite {
        read_write.push("/tmp");
        read_write.push(policy.workspace_root.as_str());
    }
    grant_args(&["/"], &read_write)
}

/// `sandbox-exec -p` profile for `policy`. Writable roots come from [`crate::writable_roots`].
#[must_use]
pub fn seatbelt_profile_args(policy: &SandboxPolicy) -> Vec<String> {
    let mut forms = vec![
        "(version 1)".into(),
        "(allow default)".into(),
        "(deny file-write*)".into(),
        format!("(allow file-write* (literal {}))", sbpl_string("/dev/null")),
    ];
    let roots = writable_roots(&SandboxExecutionPolicy {
        mode: policy.mode,
        workspace_root: policy.workspace_root.clone(),
        session_id: policy.session_id.clone(),
    });
    if !roots.is_empty() {
        let grants = roots
            .iter()
            .map(|root| format!("(subpath {})", sbpl_string(root)))
            .collect::<Vec<_>>()
            .join(" ");
        forms.push(format!("(allow file-write* {grants})"));
    }
    vec!["-p".into(), forms.join(" ")]
}

fn sbpl_string(path: &str) -> String {
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::{bwrap_profile_args, landlock_profile_args, seatbelt_profile_args};
    use crate::{SandboxMode, SandboxPolicy};

    fn ro() -> SandboxPolicy {
        SandboxPolicy {
            mode: SandboxMode::ReadOnly,
            workspace_root: "/ws".into(),
            session_id: None,
        }
    }
    fn ww() -> SandboxPolicy {
        SandboxPolicy {
            mode: SandboxMode::WorkspaceWrite,
            workspace_root: "/ws".into(),
            session_id: None,
        }
    }

    #[test]
    fn bwrap_and_landlock_profiles() {
        assert_eq!(
            bwrap_profile_args(&ro()),
            [
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--die-with-parent"
            ]
        );
        assert_eq!(
            bwrap_profile_args(&ww()),
            [
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--die-with-parent",
                "--tmpfs",
                "/tmp",
                "--bind",
                "/ws",
                "/ws"
            ]
        );
        assert_eq!(
            landlock_profile_args(&ro()),
            ["--ro", "/", "--rw", "/dev/null"]
        );
        assert_eq!(
            landlock_profile_args(&ww()),
            [
                "--ro",
                "/",
                "--rw",
                "/dev/null",
                "--rw",
                "/tmp",
                "--rw",
                "/ws"
            ]
        );
    }

    #[test]
    fn seatbelt_read_only_profile() {
        assert_eq!(
            seatbelt_profile_args(&ro()),
            [
                "-p",
                "(version 1) (allow default) (deny file-write*) (allow file-write* (literal \"/dev/null\"))"
            ]
        );
    }
}
