//! Landlock launcher path, grant argv, and functional probe.
//!
//! The launcher path is resolved from the repository layout. This module never reads
//! environment variables to choose which binary confines a process.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// File name of the Landlock launcher binary inside each platform package `bin/` directory.
pub const LAUNCHER_BIN: &str = "landlock-run";

/// Exit status the launcher uses for every launcher-level failure.
///
/// After a successful `exec`, the wrapped command may also return 125, so consumers also
/// require a matching launcher-owned fatal diagnostic.
pub const LAUNCHER_FAILURE_EXIT: i32 = 125;

/// Probe verdict for one Landlock launcher on this host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LandlockEnforcement {
    /// The running kernel enforces every access class the launcher can govern.
    Full,
    /// An older Landlock ABI governs only a subset of access classes.
    Partial,
    /// Missing binary, failed spawn, non-zero exit, timeout, or an unenforcing kernel.
    Unusable,
}

/// Absolute path of the `landlock-run` binary for this host architecture.
///
/// Candidates, first existing file wins; if none exist, the last candidate is returned so a
/// later probe is unusable:
///
/// 1. `{workspace}/native/landlock-run/packages/linux-{arch}/bin/landlock-run`
/// 2. `{workspace}/node_modules/@deepseek-ai/node-addon-landlock-run-linux-{arch}/bin/landlock-run`
///
/// `workspace` is two directories above this crate's `CARGO_MANIFEST_DIR`. `arch` maps
/// `x86_64` → `x64` and `aarch64` → `arm64`. The path is never cwd-relative and is never
/// taken from the process environment.
#[must_use]
pub fn launcher_path() -> PathBuf {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("dsh-sandbox crate lives at crates/dsh-sandbox");
    let native = workspace
        .join("native")
        .join("landlock-run")
        .join("packages")
        .join(format!("linux-{arch}"))
        .join("bin")
        .join(LAUNCHER_BIN);
    let packaged = workspace
        .join("node_modules")
        .join("@deepseek-ai")
        .join(format!("node-addon-landlock-run-linux-{arch}"))
        .join("bin")
        .join(LAUNCHER_BIN);
    if native.is_file() { native } else { packaged }
}

/// Grant argv before `--`: `--ro` for each read-only root, then `--rw` for each read-write root.
#[must_use]
pub fn grant_args(read_only: &[&str], read_write: &[&str]) -> Vec<String> {
    let mut args = Vec::with_capacity((read_only.len() + read_write.len()) * 2);
    for path in read_only {
        args.push("--ro".into());
        args.push((*path).into());
    }
    for path in read_write {
        args.push("--rw".into());
        args.push((*path).into());
    }
    args
}

/// Run `launcher --probe` and classify the host's Landlock enforcement.
///
/// stdin is null and stdout is piped. A spawn failure, non-zero exit, or wait exceeding
/// `timeout_ms` (the child is then killed) is [`LandlockEnforcement::Unusable`]. Zero exit
/// with stdout containing `partially enforced` is [`LandlockEnforcement::Partial`]; any other
/// zero exit is [`LandlockEnforcement::Full`].
#[must_use]
pub fn probe(launcher: &Path, timeout_ms: u64) -> LandlockEnforcement {
    let mut child = match Command::new(launcher)
        .arg("--probe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return LandlockEnforcement::Unusable,
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return LandlockEnforcement::Unusable;
    };
    let reader = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        buf
    });
    let Some(status) = wait_or_kill(&mut child, Duration::from_millis(timeout_ms)) else {
        return LandlockEnforcement::Unusable;
    };
    if !status.success() {
        return LandlockEnforcement::Unusable;
    }
    let stdout = reader.join().unwrap_or_default();
    if stdout.contains("partially enforced") {
        LandlockEnforcement::Partial
    } else {
        LandlockEnforcement::Full
    }
}

/// Wait for `child` up to `timeout`. On expiry, kill the child, reap it, and return [`None`].
pub(crate) fn wait_or_kill(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LAUNCHER_BIN, LAUNCHER_FAILURE_EXIT, LandlockEnforcement, grant_args, launcher_path, probe,
    };

    #[test]
    fn cli_constants_match_the_c_helper_contract() {
        assert_eq!(LAUNCHER_BIN, "landlock-run");
        assert_eq!(LAUNCHER_FAILURE_EXIT, 125);
        assert_eq!(
            grant_args(&["/"], &["/dev/null", "/tmp"]),
            ["--ro", "/", "--rw", "/dev/null", "--rw", "/tmp"]
        );
    }

    #[test]
    #[allow(unused_unsafe)]
    fn launcher_path_ignores_environment_variables() {
        let previous = std::env::var("LANDLOCK_RUN").ok();
        let decoy = "/tmp/dsh-phase4-decoy-landlock-run";
        unsafe {
            std::env::set_var("LANDLOCK_RUN", decoy);
        }
        unsafe {
            std::env::set_var("DSH_LANDLOCK_RUN", decoy);
        }
        let path = launcher_path();
        assert_ne!(path.to_string_lossy(), decoy);
        assert!(path.is_absolute(), "{path:?}");
        match previous {
            Some(value) => unsafe { std::env::set_var("LANDLOCK_RUN", value) },
            None => unsafe { std::env::remove_var("LANDLOCK_RUN") },
        }
        unsafe {
            std::env::remove_var("DSH_LANDLOCK_RUN");
        }
    }

    #[test]
    fn probe_missing_binary_is_unusable() {
        assert_eq!(
            probe(
                std::path::Path::new("/definitely-missing-dsh-landlock-run"),
                1_000
            ),
            LandlockEnforcement::Unusable
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_parses_stdout_and_times_out() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("dsh-landlock-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let full = dir.join("full");
        let partial = dir.join("partial");
        let hang = dir.join("hang");
        let failed = dir.join("failed");
        std::fs::write(
            &full,
            "#!/bin/sh\necho 'landlock: fully enforced'\nexit 0\n",
        )
        .unwrap();
        std::fs::write(
            &partial,
            "#!/bin/sh\necho 'landlock: partially enforced (older ABI)'\nexit 0\n",
        )
        .unwrap();
        std::fs::write(&hang, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::write(&failed, "#!/bin/sh\necho nope\nexit 1\n").unwrap();
        for path in [&full, &partial, &hang, &failed] {
            let mut perms = std::fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).unwrap();
        }
        assert_eq!(probe(&full, 2_000), LandlockEnforcement::Full);
        assert_eq!(probe(&partial, 2_000), LandlockEnforcement::Partial);
        assert_eq!(probe(&failed, 2_000), LandlockEnforcement::Unusable);
        assert_eq!(probe(&hang, 200), LandlockEnforcement::Unusable);
    }
}
