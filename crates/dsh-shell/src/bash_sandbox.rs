//! Sandboxed POSIX bash executor: confine argv, classify runner failure, never silently unconfine.

use std::sync::Arc;

use dsh_sandbox::{
    ConfinedArgv, LocalSandboxProvider, SandboxError, SandboxExecutionPolicy, SandboxMode,
    SandboxPolicy,
};

use crate::bash::{LocalBashExecutor, ShellProcess};
use crate::classify::{classify_denial, classify_runner_failure};
use crate::error::ShellError;
use crate::types::{ShellExecRequest, ShellExecSpec, ShellRunResult, ShellSandboxInfo};

/// Wrap caller argv in a sandbox runner.
///
/// Implementations must never return the original argv as a silent unconfined fallback.
pub trait Confine: Send + Sync {
    /// Wrap `argv` for `policy`.
    ///
    /// # Errors
    ///
    /// [`SandboxError::Unavailable`] when no usable backend exists for the requested mode.
    /// [`SandboxError::Invalid`] when `policy.mode` is [`SandboxMode::DangerFullAccess`].
    fn confine(
        &self,
        argv: &[String],
        policy: &SandboxPolicy,
    ) -> Result<ConfinedArgv, SandboxError>;
}

impl Confine for LocalSandboxProvider {
    fn confine(
        &self,
        argv: &[String],
        policy: &SandboxPolicy,
    ) -> Result<ConfinedArgv, SandboxError> {
        LocalSandboxProvider::confine(self, argv, policy)
    }
}

/// POSIX bash executor that confines argv through [`Confine`] and classifies runner failure.
///
/// [`SandboxMode::DangerFullAccess`] is the only unconfined path: [`Confine::confine`] is not
/// called. Any other mode wraps `["bash", "-c", command]` and fails closed on
/// [`dsh_sandbox::SANDBOX_UNAVAILABLE`]. Exit 125 alone is not a launcher failure; a matching
/// fatal `landlock-run: ` stderr line is also required.
pub struct SandboxBashExecutor {
    inner: LocalBashExecutor,
    sandbox: Arc<dyn Confine>,
    default_policy: SandboxExecutionPolicy,
}

impl SandboxBashExecutor {
    /// Retain `inner`, `sandbox`, and the deployment `default_policy`.
    #[must_use]
    pub fn new(
        inner: LocalBashExecutor,
        sandbox: Arc<dyn Confine>,
        default_policy: SandboxExecutionPolicy,
    ) -> Self {
        Self {
            inner,
            sandbox,
            default_policy,
        }
    }

    /// The constructed default mode. Per-call [`ShellExecSpec::sandbox_policy`] may differ.
    #[must_use]
    pub fn sandbox_mode(&self) -> Option<SandboxMode> {
        Some(self.default_policy.mode)
    }

    /// Fill local-executor defaults, then stamp `sandbox_policy`.
    ///
    /// `sandbox_policy` is `request.sandbox_policy` when present, otherwise `default_policy`.
    /// After this call the spec's policy is always [`Some`].
    ///
    /// # Errors
    ///
    /// [`ShellError::Invalid`] for the same reasons as [`LocalBashExecutor::resolve`].
    pub fn resolve(&self, request: ShellExecRequest) -> Result<ShellExecSpec, ShellError> {
        let policy = request
            .sandbox_policy
            .clone()
            .unwrap_or_else(|| self.default_policy.clone());
        let mut spec = self.inner.resolve(request)?;
        spec.sandbox_policy = Some(policy);
        Ok(spec)
    }

    /// Foreground-run under the spec's sandbox policy.
    ///
    /// [`SandboxMode::DangerFullAccess`] calls [`LocalBashExecutor::run`] and stamps
    /// [`ShellSandboxInfo`] with that mode and `denied: false` without calling
    /// [`Confine::confine`]. Any other mode confines `["bash", "-c", command]`, then
    /// [`LocalBashExecutor::run_argv`]. A matching [`classify_runner_failure`] is
    /// [`ShellError::Sandbox`] with [`dsh_sandbox::SANDBOX_UNAVAILABLE`]. Otherwise the result
    /// carries `denied`, `enforcement`, and `runner_failed: Some(false)`.
    ///
    /// Must be called from a Tokio runtime.
    ///
    /// # Errors
    ///
    /// [`ShellError::Invalid`] when `spec.sandbox_policy` is [`None`].
    /// [`ShellError::Sandbox`] when confine fails or runner-failure evidence matches.
    /// [`ShellError::Subprocess`] as in [`LocalBashExecutor::run`].
    pub async fn run(&self, spec: ShellExecSpec) -> Result<ShellRunResult, ShellError> {
        let policy = required_policy(&spec)?;
        if policy.mode == SandboxMode::DangerFullAccess {
            let mut result = self.inner.run(spec).await?;
            result.sandbox = Some(ShellSandboxInfo {
                mode: policy.mode,
                denied: false,
                enforcement: None,
                runner_failed: None,
            });
            return Ok(result);
        }
        let confined = self.confine_command(&spec, &policy)?;
        let mut result = self.inner.run_argv(&spec, &confined.argv).await?;
        if let Some(matched) = classify_runner_failure(
            result.exit_code,
            &result.stderr.text,
            &confined.runner_failure_rules,
        ) {
            return Err(SandboxError::unavailable(policy.mode, Some(&matched.detail)).into());
        }
        result.sandbox = Some(ShellSandboxInfo {
            mode: policy.mode,
            denied: classify_denial(
                result.exit_code,
                &result.stderr.text,
                &confined.denial_signatures,
            ),
            enforcement: Some(confined.enforcement),
            runner_failed: Some(false),
        });
        Ok(result)
    }

    /// Background-start under the spec's sandbox policy. Ignores `spec.timeout_ms`.
    ///
    /// [`SandboxMode::DangerFullAccess`] calls [`LocalBashExecutor::start`] without confine.
    /// Any other mode confines `["bash", "-c", command]`, then
    /// [`LocalBashExecutor::start_argv`]. Must be called from a Tokio runtime.
    ///
    /// # Errors
    ///
    /// [`ShellError::Invalid`] when `spec.sandbox_policy` is [`None`].
    /// [`ShellError::Sandbox`] when confine fails.
    /// [`ShellError::Subprocess`] as in [`LocalBashExecutor::start`].
    pub fn start(&self, spec: ShellExecSpec) -> Result<ShellProcess, ShellError> {
        let policy = required_policy(&spec)?;
        if policy.mode == SandboxMode::DangerFullAccess {
            return self.inner.start(spec);
        }
        let confined = self.confine_command(&spec, &policy)?;
        self.inner.start_argv(&spec, &confined.argv)
    }

    fn confine_command(
        &self,
        spec: &ShellExecSpec,
        policy: &SandboxExecutionPolicy,
    ) -> Result<ConfinedArgv, ShellError> {
        let confined_policy = SandboxPolicy::confined(policy.clone())?;
        Ok(self.sandbox.confine(
            &["bash".into(), "-c".into(), spec.command.clone()],
            &confined_policy,
        )?)
    }
}

fn required_policy(spec: &ShellExecSpec) -> Result<SandboxExecutionPolicy, ShellError> {
    spec.sandbox_policy.clone().ok_or_else(|| {
        ShellError::Invalid(
            "sandbox_policy is required; call SandboxBashExecutor::resolve first".into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{Confine, SandboxBashExecutor};
    use crate::{BashConfig, LocalBashExecutor, ShellExecRequest};
    use dsh_sandbox::{
        ConfinedArgv, RunnerFailureRule, SANDBOX_UNAVAILABLE, SandboxEnforcement, SandboxError,
        SandboxExecutionPolicy, SandboxMode, SandboxPolicy,
    };
    use dsh_subprocess::LocalSubprocessRuntime;
    use std::sync::Arc;

    struct FailClosed;
    impl Confine for FailClosed {
        fn confine(&self, _: &[String], _: &SandboxPolicy) -> Result<ConfinedArgv, SandboxError> {
            Err(SandboxError::unavailable(SandboxMode::ReadOnly, None))
        }
    }

    struct Recording {
        inner: ConfinedArgv,
        calls: std::sync::Mutex<Vec<Vec<String>>>,
    }
    impl Confine for Recording {
        fn confine(
            &self,
            argv: &[String],
            _: &SandboxPolicy,
        ) -> Result<ConfinedArgv, SandboxError> {
            self.calls.lock().unwrap().push(argv.to_vec());
            Ok(self.inner.clone())
        }
    }

    #[tokio::test]
    async fn unavailable_confine_does_not_run_the_command() {
        let inner = LocalBashExecutor::new(
            Arc::new(LocalSubprocessRuntime::new()),
            BashConfig::default(),
        )
        .unwrap();
        let bash = SandboxBashExecutor::new(
            inner,
            Arc::new(FailClosed),
            SandboxExecutionPolicy {
                mode: SandboxMode::ReadOnly,
                workspace_root: "/ws".into(),
                session_id: None,
            },
        );
        let spec = bash
            .resolve(ShellExecRequest {
                command: "echo should-not-run".into(),
                timeout_ms: Some(5_000),
                workdir: None,
                stdout_max_bytes: None,
                signal: None,
                stdin: None,
                env: None,
                dsh_env: None,
                sandbox_policy: None,
            })
            .unwrap();
        let err = bash.run(spec).await.unwrap_err();
        match err {
            crate::ShellError::Sandbox(s) => assert_eq!(s.code(), SANDBOX_UNAVAILABLE),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn danger_full_access_does_not_call_confine() {
        let inner = LocalBashExecutor::new(
            Arc::new(LocalSubprocessRuntime::new()),
            BashConfig::default(),
        )
        .unwrap();
        let recording = Arc::new(Recording {
            inner: ConfinedArgv {
                argv: vec!["/bin/false".into()],
                enforcement: SandboxEnforcement::Full,
                denial_signatures: vec![],
                runner_failure_rules: vec![],
            },
            calls: std::sync::Mutex::new(vec![]),
        });
        let bash = SandboxBashExecutor::new(
            inner,
            recording.clone(),
            SandboxExecutionPolicy {
                mode: SandboxMode::DangerFullAccess,
                workspace_root: "/ws".into(),
                session_id: None,
            },
        );
        let spec = bash
            .resolve(ShellExecRequest {
                command: "echo unconfined".into(),
                timeout_ms: Some(5_000),
                workdir: None,
                stdout_max_bytes: None,
                signal: None,
                stdin: None,
                env: None,
                dsh_env: None,
                sandbox_policy: None,
            })
            .unwrap();
        let result = bash.run(spec).await.unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(recording.calls.lock().unwrap().is_empty());
        assert_eq!(
            result.sandbox.as_ref().unwrap().mode,
            SandboxMode::DangerFullAccess
        );
        assert!(!result.sandbox.as_ref().unwrap().denied);
    }

    #[tokio::test]
    async fn landlock_125_fatal_line_becomes_sandbox_unavailable() {
        let dir = std::env::temp_dir().join(format!("dsh-fake-ll-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("landlock-run");
        std::fs::write(
            &fake,
            "#!/bin/sh\nprintf '%s\\n' 'landlock-run: partial enforcement (older Landlock ABI)' >&2\nprintf '%s\\n' 'landlock-run: ruleset creation failed' >&2\nexit 125\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&fake).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&fake, p).unwrap();
        }
        let inner = LocalBashExecutor::new(
            Arc::new(LocalSubprocessRuntime::new()),
            BashConfig::default(),
        )
        .unwrap();
        let recording = Arc::new(Recording {
            inner: ConfinedArgv {
                argv: vec![
                    fake.to_string_lossy().into_owned(),
                    "--".into(),
                    "bash".into(),
                    "-c".into(),
                    "echo ran".into(),
                ],
                enforcement: SandboxEnforcement::Partial,
                denial_signatures: vec!["permission denied".into()],
                runner_failure_rules: vec![RunnerFailureRule {
                    allowed_exit_codes: Some(vec![125]),
                    fatal_signatures: vec!["landlock-run: ".into()],
                    informational_lines: vec![
                        "landlock-run: partial enforcement (older Landlock ABI)".into(),
                    ],
                }],
            },
            calls: std::sync::Mutex::new(vec![]),
        });
        let bash = SandboxBashExecutor::new(
            inner,
            recording.clone(),
            SandboxExecutionPolicy {
                mode: SandboxMode::ReadOnly,
                workspace_root: "/ws".into(),
                session_id: None,
            },
        );
        let spec = bash
            .resolve(ShellExecRequest {
                command: "echo ran".into(),
                timeout_ms: Some(5_000),
                workdir: None,
                stdout_max_bytes: None,
                signal: None,
                stdin: None,
                env: None,
                dsh_env: None,
                sandbox_policy: None,
            })
            .unwrap();
        let err = bash.run(spec).await.unwrap_err();
        match err {
            crate::ShellError::Sandbox(s) => {
                assert_eq!(s.code(), SANDBOX_UNAVAILABLE);
                assert!(
                    s.to_string()
                        .contains("Runner failure: landlock-run: ruleset creation failed")
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            recording.calls.lock().unwrap()[0],
            ["bash", "-c", "echo ran"]
        );
    }
}
