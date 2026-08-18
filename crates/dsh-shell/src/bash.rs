//! Unfenced POSIX bash executor: `resolve` then `run` / `start`.
//!
//! [`LocalBashExecutor::run`] and [`LocalBashExecutor::start`] spawn
//! `["bash", "-c", command]` through [`dsh_subprocess::LocalSubprocessRuntime`].
//! They never re-default omitted request fields. [`LocalBashExecutor::sandbox_mode`]
//! is [`None`]; this executor ignores [`crate::ShellExecSpec::sandbox_policy`].

use std::future;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dsh_sandbox::SandboxMode;
use dsh_subprocess::{
    CollectedOutput, DSH_ENV_PREFIX, EnvEntry, LocalSubprocessRuntime, MAX_GRACE_MS,
    OutputCollector, SubprocessCollect, SubprocessError, SubprocessHandle, SubprocessOutput,
    SubprocessOutputRead, SubprocessSpawnSpec, SubprocessStdin, SubprocessStdio,
};
use dsh_tools::AbortFlag;
use tokio::sync::Mutex as AsyncMutex;

use crate::error::ShellError;
use crate::types::{ShellExecRequest, ShellExecSpec, ShellProcessStatus, ShellRunResult};

/// Model-friendly child-environment overlay applied before `spec.env` and `spec.dsh_env`.
///
/// Sets `NO_COLOR=1`, `TERM=dumb`, `PAGER=cat`, and `GIT_PAGER=cat`. A later `spec.env` or
/// `spec.dsh_env` entry for the same key wins. The subprocess crate still credential-scrubs
/// the parent environment.
pub const ENV_OVERRIDES: &[(&str, &str)] = &[
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
];

/// Validated budgets for [`LocalBashExecutor`].
///
/// [`LocalBashExecutor::new`] rejects a zero timeout, output, or spill cap, a zero `grace_ms`,
/// and a `grace_ms` greater than [`MAX_GRACE_MS`].
#[derive(Clone, Debug)]
pub struct BashConfig {
    /// Default working directory when the request omits `workdir`. `None` uses the process cwd.
    pub cwd: Option<String>,
    /// Default foreground timeout in milliseconds when the request omits `timeout_ms`.
    pub timeout_ms: u64,
    /// Upper bound applied to both the default timeout and a per-call override.
    pub max_timeout_ms: u64,
    /// Default per-stream in-memory capture cap when the request omits `stdout_max_bytes`.
    pub max_output_bytes: usize,
    /// Per-stream full-output spill cap passed to the subprocess collector.
    pub max_spill_bytes: usize,
    /// SIGTERM→SIGKILL grace and post-exit collect drain, in milliseconds.
    pub grace_ms: u64,
}

impl Default for BashConfig {
    fn default() -> Self {
        Self {
            cwd: None,
            timeout_ms: 120_000,
            max_timeout_ms: 600_000,
            max_output_bytes: 64_000,
            max_spill_bytes: 64 * 1024 * 1024,
            grace_ms: 3_000,
        }
    }
}

/// Unfenced POSIX bash executor over a local subprocess runtime.
pub struct LocalBashExecutor {
    /// Runtime that owns spawned process trees until they exit or it is disposed.
    pub subprocess: Arc<LocalSubprocessRuntime>,
    /// Budgets used by [`Self::resolve`] and every spawn.
    pub config: BashConfig,
}

impl LocalBashExecutor {
    /// Validate `config` and retain `subprocess` for later spawns.
    ///
    /// # Errors
    ///
    /// [`ShellError::Invalid`] when a timeout, output, or spill cap is zero, when `grace_ms` is
    /// zero, or when `grace_ms` is greater than [`MAX_GRACE_MS`].
    pub fn new(
        subprocess: Arc<LocalSubprocessRuntime>,
        config: BashConfig,
    ) -> Result<Self, ShellError> {
        validate_config(&config)?;
        Ok(Self { subprocess, config })
    }

    /// Always [`None`]: this executor does not confine.
    #[must_use]
    pub fn sandbox_mode(&self) -> Option<SandboxMode> {
        None
    }

    /// Fill `workdir`, `timeout_ms`, and `stdout_max_bytes`, and carry overlays verbatim.
    ///
    /// `workdir` is `request.workdir`, else `config.cwd`, else the process current directory.
    /// `timeout_ms` is `min(request.timeout_ms.unwrap_or(config.timeout_ms), config.max_timeout_ms)`
    /// and must be positive. `stdout_max_bytes` is `request.stdout_max_bytes` else
    /// `config.max_output_bytes` and must be positive. `sandbox_policy` is copied as-is and is
    /// not defaulted. When `dsh_env` is `Some`, every key must start with [`DSH_ENV_PREFIX`].
    ///
    /// # Errors
    ///
    /// [`ShellError::Invalid`] when the current directory cannot be read, when the resolved
    /// timeout or stdout cap is zero, or when a `dsh_env` key does not start with `DSH_`.
    pub fn resolve(&self, request: ShellExecRequest) -> Result<ShellExecSpec, ShellError> {
        let workdir = match request.workdir.or_else(|| self.config.cwd.clone()) {
            Some(dir) => dir,
            None => std::env::current_dir()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| {
                    ShellError::Invalid(format!("cannot determine current directory: {error}"))
                })?,
        };
        let timeout_ms = request
            .timeout_ms
            .unwrap_or(self.config.timeout_ms)
            .min(self.config.max_timeout_ms);
        if timeout_ms == 0 {
            return Err(ShellError::Invalid("timeout_ms must be positive".into()));
        }
        let stdout_max_bytes = request
            .stdout_max_bytes
            .unwrap_or(self.config.max_output_bytes);
        if stdout_max_bytes == 0 {
            return Err(ShellError::Invalid(
                "stdout_max_bytes must be positive".into(),
            ));
        }
        if let Some(pairs) = &request.dsh_env {
            for (key, _) in pairs {
                if !key.starts_with(DSH_ENV_PREFIX) {
                    return Err(ShellError::Invalid(format!(
                        "dsh_env key {key:?} must start with {DSH_ENV_PREFIX}"
                    )));
                }
            }
        }
        Ok(ShellExecSpec {
            command: request.command,
            workdir,
            timeout_ms,
            stdout_max_bytes,
            signal: request.signal,
            stdin: request.stdin,
            env: request.env,
            dsh_env: request.dsh_env,
            sandbox_policy: request.sandbox_policy,
        })
    }

    /// Foreground-run `["bash", "-c", spec.command]`.
    ///
    /// Must be called from a Tokio runtime. Nonzero command exits resolve as
    /// [`Ok`](Result::Ok) with [`ShellRunResult::exit_code`] set. Timeout and abort set
    /// [`ShellRunResult::timed_out`] or [`ShellRunResult::aborted`] (mutually exclusive) and
    /// still return [`Ok`](Result::Ok). [`ShellRunResult::sandbox`] is [`None`].
    ///
    /// # Errors
    ///
    /// [`ShellError::Subprocess`] when spawn fails, including an already-aborted
    /// [`ShellExecSpec::signal`], or when waiting on the child fails after spawn.
    pub async fn run(&self, spec: ShellExecSpec) -> Result<ShellRunResult, ShellError> {
        let argv = bash_argv(&spec.command);
        self.run_argv(&spec, &argv).await
    }

    /// Background-start `["bash", "-c", spec.command]`. Ignores `spec.timeout_ms`.
    ///
    /// Must be called from a Tokio runtime. Spawn failure is [`Err`] here; [`ShellProcess::done`]
    /// never fails the future.
    ///
    /// # Errors
    ///
    /// [`ShellError::Subprocess`] when spawn fails, including an already-aborted
    /// [`ShellExecSpec::signal`].
    pub fn start(&self, spec: ShellExecSpec) -> Result<ShellProcess, ShellError> {
        let argv = bash_argv(&spec.command);
        self.start_argv(&spec, &argv)
    }

    /// Foreground-run an explicit argv with this executor's env, stdio, timeout, and abort race.
    ///
    /// `argv` is passed through unchanged; it is never joined into a shell string. Must be called
    /// from a Tokio runtime.
    ///
    /// # Errors
    ///
    /// Same as [`Self::run`].
    pub async fn run_argv(
        &self,
        spec: &ShellExecSpec,
        argv: &[String],
    ) -> Result<ShellRunResult, ShellError> {
        if spec.signal.as_ref().is_some_and(AbortFlag::is_aborted) {
            return Err(SubprocessError::AbortedBeforeSpawn.into());
        }
        let handle = self.subprocess.spawn(self.spawn_spec(spec, argv, None))?;
        let abort = spec.signal.clone();
        let timeout_ms = spec.timeout_ms;
        let (outcome, timed_out, aborted) = tokio::select! {
            result = handle.done() => (result?, false, false),
            () = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                handle.terminate();
                (handle.done().await?, true, false)
            }
            () = async {
                match abort {
                    Some(flag) => flag.cancelled().await,
                    None => future::pending().await,
                }
            } => {
                handle.terminate();
                (handle.done().await?, false, true)
            }
        };
        let (stdout, stderr) = take_collected(&handle).await?;
        Ok(ShellRunResult {
            exit_code: outcome.exit_code,
            signal: outcome.signal,
            timed_out,
            aborted,
            timeout_ms: spec.timeout_ms,
            stdout,
            stderr,
            sandbox: None,
        })
    }

    /// Background-start an explicit argv. Ignores `spec.timeout_ms`; callers stop the tree with
    /// [`ShellProcess::kill`] or [`ShellExecSpec::signal`].
    ///
    /// Must be called from a Tokio runtime.
    ///
    /// # Errors
    ///
    /// Same as [`Self::start`].
    pub fn start_argv(
        &self,
        spec: &ShellExecSpec,
        argv: &[String],
    ) -> Result<ShellProcess, ShellError> {
        let handle = self
            .subprocess
            .spawn(self.spawn_spec(spec, argv, spec.signal.clone()))?;
        Ok(ShellProcess::from_handle(handle))
    }

    fn spawn_spec(
        &self,
        spec: &ShellExecSpec,
        argv: &[String],
        signal: Option<AbortFlag>,
    ) -> SubprocessSpawnSpec {
        let collect = SubprocessCollect {
            max_bytes: spec.stdout_max_bytes,
            spill_max_bytes: Some(self.config.max_spill_bytes),
        };
        SubprocessSpawnSpec {
            argv: argv.to_vec(),
            cwd: spec.workdir.clone(),
            stdio: SubprocessStdio {
                stdin: match &spec.stdin {
                    Some(data) => SubprocessStdin::Data(data.clone()),
                    None => SubprocessStdin::Ignore,
                },
                stdout: SubprocessOutput::Collect(collect.clone()),
                stderr: SubprocessOutput::Collect(collect),
            },
            grace_ms: self.config.grace_ms,
            signal,
            env: Some(merged_env(spec)),
        }
    }
}

/// Background process handle returned by [`LocalBashExecutor::start`].
///
/// Consecutive [`Self::read_output`] calls never re-deliver bytes. [`Self::done`] waits until the
/// tree has closed and never fails the future.
pub struct ShellProcess {
    inner: Arc<ShellProcessInner>,
}

struct ShellProcessInner {
    handle: SubprocessHandle,
    status: AtomicU8,
    stdout_offset: Mutex<u64>,
    stderr_offset: Mutex<u64>,
}

/// One consuming [`ShellProcess::read_output`] read.
///
/// `delta` is new stdout then, when stderr produced bytes, a `[stderr]` section. `lossy` is true
/// when either stream dropped unread bytes from its in-memory tail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellProcessRead {
    /// Output produced since the previous read (stdout first).
    pub delta: String,
    /// True when truncation dropped unread bytes the delta cannot include.
    pub lossy: bool,
    /// Full stdout spill file, when one was created and remains intact.
    pub stdout_spill_path: Option<String>,
    /// Full stderr spill file, when one was created and remains intact.
    pub stderr_spill_path: Option<String>,
}

const STATUS_RUNNING: u8 = 0;
const STATUS_COMPLETED: u8 = 1;
const STATUS_KILLED: u8 = 2;

impl ShellProcess {
    fn from_handle(handle: SubprocessHandle) -> Self {
        let inner = Arc::new(ShellProcessInner {
            handle,
            status: AtomicU8::new(STATUS_RUNNING),
            stdout_offset: Mutex::new(0),
            stderr_offset: Mutex::new(0),
        });
        let waiter = Arc::clone(&inner);
        tokio::spawn(async move {
            settle_inner(&waiter).await;
        });
        Self { inner }
    }

    /// Current lifecycle state. Settles to [`ShellProcessStatus::Completed`] or
    /// [`ShellProcessStatus::Killed`] at most once.
    #[must_use]
    pub fn status(&self) -> ShellProcessStatus {
        decode_status(self.inner.status.load(Ordering::Acquire))
    }

    /// Terminate the process group. Returns `false` when the process had already finished.
    #[must_use]
    pub fn kill(&self) -> bool {
        match self.inner.status.compare_exchange(
            STATUS_RUNNING,
            STATUS_KILLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                self.inner.handle.terminate();
                true
            }
            Err(_) => false,
        }
    }

    /// Wait until the underlying process tree has closed. Never fails the future.
    pub async fn done(&self) {
        settle_inner(&self.inner).await;
    }

    /// Read output produced since the previous read. Consecutive calls never re-deliver.
    #[must_use]
    pub fn read_output(&self) -> ShellProcessRead {
        let mut stdout_offset = lock(&self.inner.stdout_offset);
        let mut stderr_offset = lock(&self.inner.stderr_offset);
        let stdout = read_delta(self.inner.handle.stdout_reader(), &mut stdout_offset);
        let stderr = read_delta(self.inner.handle.stderr_reader(), &mut stderr_offset);
        let separator = if !stdout.text.is_empty() && !stdout.text.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let delta = if stderr.text.is_empty() {
            stdout.text
        } else {
            format!("{}{separator}[stderr]\n{}", stdout.text, stderr.text)
        };
        ShellProcessRead {
            delta,
            lossy: stdout.lossy || stderr.lossy,
            stdout_spill_path: stdout.spill_path,
            stderr_spill_path: stderr.spill_path,
        }
    }
}

fn bash_argv(command: &str) -> Vec<String> {
    vec!["bash".into(), "-c".into(), command.to_owned()]
}

fn validate_config(config: &BashConfig) -> Result<(), ShellError> {
    require_positive_u64("timeout_ms", config.timeout_ms)?;
    require_positive_u64("max_timeout_ms", config.max_timeout_ms)?;
    require_positive_usize("max_output_bytes", config.max_output_bytes)?;
    require_positive_usize("max_spill_bytes", config.max_spill_bytes)?;
    if config.grace_ms == 0 || config.grace_ms > MAX_GRACE_MS {
        return Err(ShellError::Invalid(format!(
            "grace_ms must be a positive number no greater than {MAX_GRACE_MS}"
        )));
    }
    Ok(())
}

fn require_positive_u64(name: &str, value: u64) -> Result<(), ShellError> {
    if value == 0 {
        return Err(ShellError::Invalid(format!("{name} must be positive")));
    }
    Ok(())
}

fn require_positive_usize(name: &str, value: usize) -> Result<(), ShellError> {
    if value == 0 {
        return Err(ShellError::Invalid(format!("{name} must be positive")));
    }
    Ok(())
}

fn merged_env(spec: &ShellExecSpec) -> Vec<EnvEntry> {
    let mut env: Vec<EnvEntry> = ENV_OVERRIDES
        .iter()
        .map(|(key, value)| EnvEntry {
            key: (*key).to_owned(),
            value: Some((*value).to_owned()),
        })
        .collect();
    if let Some(overlay) = &spec.env {
        env.extend(overlay.iter().cloned());
    }
    if let Some(dsh_env) = &spec.dsh_env {
        env.extend(dsh_env.iter().map(|(key, value)| EnvEntry {
            key: key.clone(),
            value: Some(value.clone()),
        }));
    }
    env
}

async fn take_collected(
    handle: &SubprocessHandle,
) -> Result<(CollectedOutput, CollectedOutput), ShellError> {
    Ok((
        finalize_stream(handle.stdout_reader()).await?,
        finalize_stream(handle.stderr_reader()).await?,
    ))
}

async fn finalize_stream(
    reader: Option<Arc<AsyncMutex<OutputCollector>>>,
) -> Result<CollectedOutput, ShellError> {
    let Some(reader) = reader else {
        return Err(ShellError::Invalid(
            "subprocess dropped a requested collect stream".into(),
        ));
    };
    Ok(reader.lock().await.finalize())
}

async fn settle_inner(inner: &ShellProcessInner) {
    let next = match inner.handle.done().await {
        Ok(outcome) if outcome.signal.is_none() => STATUS_COMPLETED,
        _ => STATUS_KILLED,
    };
    let _ =
        inner
            .status
            .compare_exchange(STATUS_RUNNING, next, Ordering::AcqRel, Ordering::Acquire);
}

fn decode_status(raw: u8) -> ShellProcessStatus {
    match raw {
        STATUS_COMPLETED => ShellProcessStatus::Completed,
        STATUS_KILLED => ShellProcessStatus::Killed,
        _ => ShellProcessStatus::Running,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read_delta(
    reader: Option<Arc<AsyncMutex<OutputCollector>>>,
    offset: &mut u64,
) -> SubprocessOutputRead {
    let Some(reader) = reader else {
        return SubprocessOutputRead {
            text: String::new(),
            next_offset: *offset,
            lossy: false,
            spill_path: None,
        };
    };
    let Ok(guard) = reader.try_lock() else {
        return SubprocessOutputRead {
            text: String::new(),
            next_offset: *offset,
            lossy: false,
            spill_path: None,
        };
    };
    let read = guard.read_from(*offset);
    *offset = read.next_offset;
    read
}

#[cfg(test)]
mod tests {
    use super::{BashConfig, LocalBashExecutor};
    use crate::ShellExecRequest;
    use dsh_subprocess::LocalSubprocessRuntime;
    use std::sync::Arc;

    fn exec() -> LocalBashExecutor {
        LocalBashExecutor::new(
            Arc::new(LocalSubprocessRuntime::new()),
            BashConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn resolve_fills_defaults_and_does_not_redefault_in_the_spec() {
        let spec = exec()
            .resolve(ShellExecRequest {
                command: "true".into(),
                workdir: None,
                timeout_ms: None,
                stdout_max_bytes: None,
                signal: None,
                stdin: None,
                env: None,
                dsh_env: None,
                sandbox_policy: None,
            })
            .unwrap();
        assert_eq!(spec.command, "true");
        assert_eq!(spec.timeout_ms, 120_000);
        assert_eq!(spec.stdout_max_bytes, 64_000);
        assert!(spec.sandbox_policy.is_none());
        assert!(exec().sandbox_mode().is_none());
    }

    #[test]
    fn resolve_caps_timeout() {
        let spec = exec()
            .resolve(ShellExecRequest {
                command: "true".into(),
                timeout_ms: Some(999_999),
                workdir: None,
                stdout_max_bytes: None,
                signal: None,
                stdin: None,
                env: None,
                dsh_env: None,
                sandbox_policy: None,
            })
            .unwrap();
        assert_eq!(spec.timeout_ms, 600_000);
    }

    #[tokio::test]
    async fn run_echo_and_nonzero_exit_are_results_not_errors() {
        let bash = exec();
        let spec = bash
            .resolve(ShellExecRequest {
                command: "echo phase4-bash".into(),
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
        assert!(result.stdout.text.contains("phase4-bash"));
        assert!(!result.timed_out);
        assert!(!result.aborted);

        let spec = bash
            .resolve(ShellExecRequest {
                command: "exit 3".into(),
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
        assert_eq!(result.exit_code, Some(3));
    }

    #[tokio::test]
    async fn timeout_sets_timed_out_and_kills() {
        let bash = exec();
        let spec = bash
            .resolve(ShellExecRequest {
                command: "sleep 30".into(),
                timeout_ms: Some(200),
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
        assert!(result.timed_out);
        assert!(!result.aborted);
    }

    fn blank_request(command: &str) -> ShellExecRequest {
        ShellExecRequest {
            command: command.into(),
            workdir: None,
            timeout_ms: None,
            stdout_max_bytes: None,
            signal: None,
            stdin: None,
            env: None,
            dsh_env: None,
            sandbox_policy: None,
        }
    }

    #[test]
    fn new_rejects_zero_grace() {
        let result = LocalBashExecutor::new(
            Arc::new(LocalSubprocessRuntime::new()),
            BashConfig {
                grace_ms: 0,
                ..BashConfig::default()
            },
        );
        assert!(matches!(result, Err(crate::ShellError::Invalid(_))));
    }

    #[test]
    fn resolve_rejects_dsh_env_without_prefix() {
        let mut request = blank_request("true");
        request.dsh_env = Some(vec![("NOT_DSH".into(), "x".into())]);
        let err = exec().resolve(request).unwrap_err();
        assert!(matches!(err, crate::ShellError::Invalid(_)));
    }

    #[tokio::test]
    async fn abort_sets_aborted_and_kills() {
        let bash = exec();
        let flag = dsh_tools::AbortFlag::new();
        let mut request = blank_request("sleep 30");
        request.timeout_ms = Some(30_000);
        request.signal = Some(flag.clone());
        let spec = bash.resolve(request).unwrap();
        let run = bash.run(spec);
        tokio::pin!(run);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut run)
                .await
                .is_err(),
            "sleep 30 should still be running before abort"
        );
        flag.abort();
        let result = run.await.unwrap();
        assert!(result.aborted);
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn start_kill_and_read_output_consume() {
        let bash = exec();
        let spec = bash
            .resolve(blank_request("printf 'phase4-start\\n'; sleep 30"))
            .unwrap();
        let proc = bash.start(spec).unwrap();
        assert_eq!(proc.status(), crate::ShellProcessStatus::Running);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let first = proc.read_output();
        assert!(
            first.delta.contains("phase4-start"),
            "delta={:?}",
            first.delta
        );
        let second = proc.read_output();
        assert!(!second.delta.contains("phase4-start"));
        assert!(proc.kill());
        proc.done().await;
        assert!(!proc.kill());
        assert_eq!(proc.status(), crate::ShellProcessStatus::Killed);
    }

    #[tokio::test]
    async fn run_applies_term_dumb_override() {
        let bash = exec();
        let spec = bash
            .resolve(blank_request(r#"printf '%s' "$TERM""#))
            .unwrap();
        let result = bash.run(spec).await.unwrap();
        assert_eq!(result.stdout.text, "dumb");
        assert!(result.sandbox.is_none());
    }
}
