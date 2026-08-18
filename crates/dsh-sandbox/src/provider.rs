//! Local sandbox provider: platform chain selection, fail-closed `confine`, and runner overrides.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use crate::error::SandboxError;
use crate::landlock::{
    LAUNCHER_BIN, LAUNCHER_FAILURE_EXIT, LandlockEnforcement, launcher_path, probe, wait_or_kill,
};
use crate::profiles::{bwrap_profile_args, landlock_profile_args, seatbelt_profile_args};
use crate::types::{
    ConfinedArgv, RunnerFailureRule, SandboxEnforcement, SandboxMode, SandboxPolicy,
};

/// Selected confinement backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunnerKind {
    /// Linux bubblewrap (`bwrap`) mount profile.
    Bwrap,
    /// Landlock self-restrict-then-exec launcher.
    Landlock,
    /// macOS `sandbox-exec` Seatbelt profile.
    Seatbelt,
}

/// Test hooks that replace platform detection, the runner chain, and functional probes.
///
/// Tests mutate these fields after [`LocalSandboxProvider::new`] and before the first
/// [`LocalSandboxProvider::confine`]. The first `confine` caches the chain verdict.
#[derive(Clone, Debug, Default)]
pub struct SandboxInternals {
    /// Replaces `std::env::consts::OS` mapping (`linux` / `darwin` / `win32`).
    pub platform: Option<String>,
    /// Replaces the platform's runner chain. An empty vector fails closed.
    pub chain: Option<Vec<RunnerKind>>,
    /// Replaces the functional `bwrap` probe.
    pub probe_bwrap: Option<fn() -> bool>,
    /// Replaces the functional Landlock launcher probe.
    pub probe_landlock: Option<fn(&Path) -> LandlockEnforcement>,
    /// Replaces the resolved `landlock-run` path.
    pub landlock_launcher: Option<PathBuf>,
}

/// Constructor configuration for [`LocalSandboxProvider`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSandboxConfig {
    /// Override runner argv prefix; non-empty skips chain selection and probing.
    pub runner_command: Vec<String>,
    /// Fatal stderr substrings for a configured [`Self::runner_command`].
    pub runner_failure_signatures: Vec<String>,
    /// Positive timeout in milliseconds for each functional probe. Default `5_000`.
    pub probe_timeout_ms: u64,
}

impl Default for LocalSandboxConfig {
    fn default() -> Self {
        Self {
            runner_command: Vec::new(),
            runner_failure_signatures: Vec::new(),
            probe_timeout_ms: 5_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Selected {
    kind: RunnerKind,
    enforcement: SandboxEnforcement,
}

/// Local process-sandbox provider. Caches the chain verdict for the provider lifetime.
#[derive(Debug)]
pub struct LocalSandboxProvider {
    /// Injectable platform, chain, and probe hooks. Mutate only before the first `confine`.
    pub internals: SandboxInternals,
    runner_command: Option<Vec<String>>,
    runner_failure_signatures: Vec<String>,
    probe_timeout_ms: u64,
    selected: Mutex<Option<Result<Selected, ()>>>,
}

impl LocalSandboxProvider {
    /// Validate `config` and build a provider with an empty chain-verdict cache.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Invalid`] when `runner_command` and `runner_failure_signatures`
    /// are unpaired, or when `probe_timeout_ms` is `0`.
    pub fn new(config: LocalSandboxConfig) -> Result<Self, SandboxError> {
        let LocalSandboxConfig {
            runner_command,
            runner_failure_signatures,
            probe_timeout_ms,
        } = config;
        if runner_command.is_empty() && !runner_failure_signatures.is_empty() {
            return Err(SandboxError::Invalid(
                "sandbox-local: runnerFailureSignatures requires runnerCommand".into(),
            ));
        }
        if !runner_command.is_empty() && runner_failure_signatures.is_empty() {
            return Err(SandboxError::Invalid(
                "sandbox-local: runnerCommand requires at least one runnerFailureSignatures entry"
                    .into(),
            ));
        }
        if probe_timeout_ms == 0 {
            return Err(SandboxError::Invalid(
                "sandbox-local: probeTimeoutMs must be a positive finite number".into(),
            ));
        }
        Ok(Self {
            internals: SandboxInternals::default(),
            runner_command: if runner_command.is_empty() {
                None
            } else {
                Some(runner_command)
            },
            runner_failure_signatures,
            probe_timeout_ms,
            selected: Mutex::new(None),
        })
    }

    /// Wrap `argv` in the selected runner for `policy`, or in `runner_command` when configured.
    ///
    /// Linux probes `bwrap` then Landlock. A sole chain candidate is selected without probing
    /// (static [`SandboxEnforcement::Full`]). An empty chain or every unusable probe returns
    /// [`SandboxError::unavailable`] and never the original argv.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Invalid`] when `policy.mode` is [`SandboxMode::DangerFullAccess`].
    /// Returns [`SandboxError::Unavailable`] when no usable runner exists.
    pub fn confine(
        &self,
        argv: &[String],
        policy: &SandboxPolicy,
    ) -> Result<ConfinedArgv, SandboxError> {
        if policy.mode == SandboxMode::DangerFullAccess {
            return Err(SandboxError::Invalid(
                "danger-full-access cannot be confined; callers must not call confine() for that mode"
                    .into(),
            ));
        }
        if let Some(runner) = &self.runner_command {
            return Ok(wrap_override(
                runner,
                argv,
                policy,
                &self.runner_failure_signatures,
            ));
        }
        let selected = self.select(policy.mode)?;
        Ok(self.wrap_selected(selected, argv, policy))
    }

    fn select(&self, mode: SandboxMode) -> Result<Selected, SandboxError> {
        let mut guard = self
            .selected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = guard.as_ref() {
            return match cached {
                Ok(selected) => Ok(selected.clone()),
                Err(()) => Err(SandboxError::unavailable(mode, None)),
            };
        }
        let verdict = self.chain_verdict();
        *guard = Some(verdict.clone());
        match verdict {
            Ok(selected) => Ok(selected),
            Err(()) => Err(SandboxError::unavailable(mode, None)),
        }
    }

    fn chain_verdict(&self) -> Result<Selected, ()> {
        let chain = self.resolved_chain();
        match chain.as_slice() {
            [] => Err(()),
            [sole] => Ok(Selected {
                kind: *sole,
                enforcement: SandboxEnforcement::Full,
            }),
            _ => {
                for kind in chain {
                    if let Some(enforcement) = self.probe_runner(kind) {
                        return Ok(Selected { kind, enforcement });
                    }
                }
                Err(())
            }
        }
    }

    fn resolved_chain(&self) -> Vec<RunnerKind> {
        if let Some(chain) = &self.internals.chain {
            return chain.clone();
        }
        let platform = match self.internals.platform.as_deref() {
            Some(platform) => platform,
            None => host_platform(),
        };
        match platform {
            "linux" => vec![RunnerKind::Bwrap, RunnerKind::Landlock],
            "darwin" => vec![RunnerKind::Seatbelt],
            _ => Vec::new(),
        }
    }

    fn probe_runner(&self, kind: RunnerKind) -> Option<SandboxEnforcement> {
        match kind {
            RunnerKind::Bwrap => {
                let usable = match self.internals.probe_bwrap {
                    Some(probe_bwrap) => probe_bwrap(),
                    None => default_probe_bwrap(self.probe_timeout_ms),
                };
                usable.then_some(SandboxEnforcement::Full)
            }
            RunnerKind::Landlock => {
                let launcher = self.landlock_launcher();
                let verdict = match self.internals.probe_landlock {
                    Some(probe_landlock) => probe_landlock(&launcher),
                    None => probe(&launcher, self.probe_timeout_ms),
                };
                match verdict {
                    LandlockEnforcement::Full => Some(SandboxEnforcement::Full),
                    LandlockEnforcement::Partial => Some(SandboxEnforcement::Partial),
                    LandlockEnforcement::Unusable => None,
                }
            }
            RunnerKind::Seatbelt => {
                default_probe_seatbelt(self.probe_timeout_ms).then_some(SandboxEnforcement::Full)
            }
        }
    }

    fn landlock_launcher(&self) -> PathBuf {
        self.internals
            .landlock_launcher
            .clone()
            .unwrap_or_else(launcher_path)
    }

    fn wrap_selected(
        &self,
        selected: Selected,
        argv: &[String],
        policy: &SandboxPolicy,
    ) -> ConfinedArgv {
        let (mut prefix, denial_signatures, runner_failure_rules) = match selected.kind {
            RunnerKind::Bwrap => (
                {
                    let mut prefix = vec!["bwrap".into()];
                    prefix.extend(bwrap_profile_args(policy));
                    prefix
                },
                vec!["read-only file system".into()],
                vec![RunnerFailureRule {
                    allowed_exit_codes: None,
                    fatal_signatures: vec!["bwrap: ".into()],
                    informational_lines: Vec::new(),
                }],
            ),
            RunnerKind::Landlock => (
                {
                    let mut prefix = vec![self.landlock_launcher().to_string_lossy().into_owned()];
                    prefix.extend(landlock_profile_args(policy));
                    prefix
                },
                vec!["permission denied".into()],
                vec![RunnerFailureRule {
                    allowed_exit_codes: Some(vec![LAUNCHER_FAILURE_EXIT]),
                    fatal_signatures: vec![format!("{LAUNCHER_BIN}: ")],
                    informational_lines: vec![format!(
                        "{LAUNCHER_BIN}: partial enforcement (older Landlock ABI)"
                    )],
                }],
            ),
            RunnerKind::Seatbelt => (
                {
                    let mut prefix = vec!["sandbox-exec".into()];
                    prefix.extend(seatbelt_profile_args(policy));
                    prefix
                },
                vec!["operation not permitted".into()],
                vec![RunnerFailureRule {
                    allowed_exit_codes: None,
                    fatal_signatures: vec!["sandbox-exec: ".into()],
                    informational_lines: Vec::new(),
                }],
            ),
        };
        prefix.push("--".into());
        prefix.extend(argv.iter().cloned());
        ConfinedArgv {
            argv: prefix,
            enforcement: selected.enforcement,
            denial_signatures,
            runner_failure_rules,
        }
    }
}

fn wrap_override(
    runner: &[String],
    argv: &[String],
    policy: &SandboxPolicy,
    signatures: &[String],
) -> ConfinedArgv {
    let mut wrapped = runner.to_vec();
    wrapped.extend(bwrap_profile_args(policy));
    wrapped.push("--".into());
    wrapped.extend(argv.iter().cloned());
    ConfinedArgv {
        argv: wrapped,
        enforcement: SandboxEnforcement::Full,
        denial_signatures: vec!["read-only file system".into(), "permission denied".into()],
        runner_failure_rules: vec![RunnerFailureRule {
            allowed_exit_codes: None,
            fatal_signatures: signatures.to_vec(),
            informational_lines: Vec::new(),
        }],
    }
}

fn host_platform() -> &'static str {
    match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

fn default_probe_bwrap(timeout_ms: u64) -> bool {
    let mut child = match Command::new("bwrap")
        .args([
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--die-with-parent",
            "--",
            "true",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    matches!(
        wait_or_kill(&mut child, Duration::from_millis(timeout_ms)),
        Some(status) if status.success()
    )
}

fn default_probe_seatbelt(timeout_ms: u64) -> bool {
    let policy = SandboxPolicy {
        mode: SandboxMode::ReadOnly,
        workspace_root: "/".into(),
        session_id: None,
    };
    let mut args = seatbelt_profile_args(&policy);
    args.push("--".into());
    args.push("true".into());
    let mut child = match Command::new("sandbox-exec")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    matches!(
        wait_or_kill(&mut child, Duration::from_millis(timeout_ms)),
        Some(status) if status.success()
    )
}

#[cfg(test)]
mod tests {
    use super::{LocalSandboxConfig, LocalSandboxProvider, RunnerKind, SandboxInternals};
    use crate::{LandlockEnforcement, SANDBOX_UNAVAILABLE, SandboxMode, SandboxPolicy};

    fn ro() -> SandboxPolicy {
        SandboxPolicy {
            mode: SandboxMode::ReadOnly,
            workspace_root: "/ws".into(),
            session_id: None,
        }
    }

    #[test]
    fn empty_chain_fails_closed() {
        let mut p = LocalSandboxProvider::new(LocalSandboxConfig::default()).unwrap();
        p.internals = SandboxInternals {
            platform: Some("linux".into()),
            chain: Some(vec![]),
            probe_bwrap: None,
            probe_landlock: None,
            landlock_launcher: None,
        };
        let err = p.confine(&["true".into()], &ro()).unwrap_err();
        assert_eq!(err.code(), SANDBOX_UNAVAILABLE);
    }

    #[test]
    fn both_linux_probes_unusable_fails_closed() {
        let mut p = LocalSandboxProvider::new(LocalSandboxConfig::default()).unwrap();
        p.internals.probe_bwrap = Some(|| false);
        p.internals.probe_landlock = Some(|_| LandlockEnforcement::Unusable);
        p.internals.platform = Some("linux".into());
        let err = p.confine(&["true".into()], &ro()).unwrap_err();
        assert_eq!(err.code(), SANDBOX_UNAVAILABLE);
    }

    #[test]
    fn landlock_wrap_includes_separator_and_failure_rule() {
        let mut p = LocalSandboxProvider::new(LocalSandboxConfig::default()).unwrap();
        p.internals.probe_bwrap = Some(|| false);
        p.internals.probe_landlock = Some(|_| LandlockEnforcement::Full);
        p.internals.landlock_launcher = Some("/opt/landlock-run".into());
        p.internals.platform = Some("linux".into());
        let wrapped = p
            .confine(&["bash".into(), "-c".into(), "true".into()], &ro())
            .unwrap();
        assert_eq!(wrapped.argv[0], "/opt/landlock-run");
        assert!(wrapped.argv.iter().any(|a| a == "--"));
        let sep = wrapped.argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(&wrapped.argv[sep + 1..], ["bash", "-c", "true"]);
        assert_eq!(wrapped.denial_signatures, ["permission denied"]);
        assert_eq!(
            wrapped.runner_failure_rules[0]
                .allowed_exit_codes
                .as_deref(),
            Some(&[125][..])
        );
        assert_eq!(
            wrapped.runner_failure_rules[0].fatal_signatures,
            ["landlock-run: "]
        );
        assert_eq!(
            wrapped.runner_failure_rules[0].informational_lines,
            ["landlock-run: partial enforcement (older Landlock ABI)"]
        );
    }

    #[test]
    fn runner_command_override_skips_probes() {
        let p = LocalSandboxProvider::new(LocalSandboxConfig {
            runner_command: vec!["/bin/echo".into()],
            runner_failure_signatures: vec!["echo-runner: ".into()],
            probe_timeout_ms: 5_000,
        })
        .unwrap();
        let wrapped = p.confine(&["true".into()], &ro()).unwrap();
        assert_eq!(wrapped.argv[0], "/bin/echo");
        assert_eq!(wrapped.enforcement, crate::SandboxEnforcement::Full);
        assert_eq!(
            wrapped.runner_failure_rules[0].fatal_signatures,
            ["echo-runner: "]
        );
    }

    #[test]
    fn runner_command_without_signatures_is_invalid() {
        let err = LocalSandboxProvider::new(LocalSandboxConfig {
            runner_command: vec!["/bin/echo".into()],
            runner_failure_signatures: vec![],
            probe_timeout_ms: 5_000,
        })
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("runnerCommand requires at least one runnerFailureSignatures entry")
        );
    }

    #[test]
    fn signatures_without_command_are_invalid() {
        let err = LocalSandboxProvider::new(LocalSandboxConfig {
            runner_command: vec![],
            runner_failure_signatures: vec!["echo-runner: ".into()],
            probe_timeout_ms: 5_000,
        })
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("runnerFailureSignatures requires runnerCommand")
        );
    }

    #[test]
    fn zero_probe_timeout_is_invalid() {
        let err = LocalSandboxProvider::new(LocalSandboxConfig {
            runner_command: vec![],
            runner_failure_signatures: vec![],
            probe_timeout_ms: 0,
        })
        .unwrap_err();
        assert!(err.to_string().contains("positive finite number"));
    }

    #[test]
    fn bwrap_wrap_includes_separator_and_failure_rule() {
        let mut p = LocalSandboxProvider::new(LocalSandboxConfig::default()).unwrap();
        p.internals.probe_bwrap = Some(|| true);
        p.internals.probe_landlock = Some(|_| panic!("landlock must not be probed"));
        p.internals.platform = Some("linux".into());
        let wrapped = p.confine(&["true".into()], &ro()).unwrap();
        assert_eq!(wrapped.argv[0], "bwrap");
        assert!(wrapped.argv.iter().any(|a| a == "--"));
        assert_eq!(wrapped.enforcement, crate::SandboxEnforcement::Full);
        assert_eq!(wrapped.denial_signatures, ["read-only file system"]);
        assert_eq!(
            wrapped.runner_failure_rules[0].fatal_signatures,
            ["bwrap: "]
        );
        assert!(wrapped.runner_failure_rules[0].allowed_exit_codes.is_none());
    }

    #[test]
    fn darwin_selects_seatbelt_without_probing() {
        let mut p = LocalSandboxProvider::new(LocalSandboxConfig::default()).unwrap();
        p.internals.platform = Some("darwin".into());
        p.internals.probe_bwrap = Some(|| panic!("bwrap must not be probed"));
        p.internals.probe_landlock = Some(|_| panic!("landlock must not be probed"));
        let wrapped = p.confine(&["true".into()], &ro()).unwrap();
        assert_eq!(wrapped.argv[0], "sandbox-exec");
        assert_eq!(wrapped.enforcement, crate::SandboxEnforcement::Full);
        assert_eq!(wrapped.denial_signatures, ["operation not permitted"]);
        assert_eq!(
            wrapped.runner_failure_rules[0].fatal_signatures,
            ["sandbox-exec: "]
        );
    }

    #[test]
    fn danger_full_access_does_not_unconfine() {
        let p = LocalSandboxProvider::new(LocalSandboxConfig::default()).unwrap();
        let policy = SandboxPolicy {
            mode: SandboxMode::DangerFullAccess,
            workspace_root: "/ws".into(),
            session_id: None,
        };
        let err = p.confine(&["true".into()], &policy).unwrap_err();
        assert_ne!(err.code(), SANDBOX_UNAVAILABLE);
        assert!(err.to_string().contains("danger-full-access"));
    }

    #[test]
    fn sole_landlock_candidate_is_selected_without_probing() {
        let mut p = LocalSandboxProvider::new(LocalSandboxConfig::default()).unwrap();
        p.internals = SandboxInternals {
            platform: Some("linux".into()),
            chain: Some(vec![RunnerKind::Landlock]),
            probe_bwrap: Some(|| panic!("bwrap must not be probed")),
            probe_landlock: Some(|_| panic!("landlock must not be probed")),
            landlock_launcher: Some("/opt/landlock-run".into()),
        };
        let wrapped = p.confine(&["true".into()], &ro()).unwrap();
        assert_eq!(wrapped.argv[0], "/opt/landlock-run");
        assert_eq!(wrapped.enforcement, crate::SandboxEnforcement::Full);
    }
}
