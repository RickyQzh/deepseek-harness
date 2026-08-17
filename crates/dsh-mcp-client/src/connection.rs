//! Connection supervisor: owns MCP stdio generations for one plugin instance.
//!
//! After a successful initialize, a child exit or read EOF starts bounded
//! exponential backoff when reconnect is enabled. One outage shares one
//! `maxAttempts` budget. Uptime past `maxDelayMs` resets the budget before
//! the next increment. Exhaustion unregisters owned public names and stops.
//! Disposal cancels backoff, kills the child, waits up to 5000 ms,
//! unregisters remaining names, and aborts the supervisor task.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use dsh_subprocess::EnvEntry;
use dsh_tools::ToolRuntime;
use serde_json::Value;
use tokio::process::Child;
use tokio::sync::{Mutex as AsyncMutex, Notify, oneshot};

use crate::spawn_stdio;
use crate::sync::swap_generation;

/// Largest delay `resolve_reconnect_policy` accepts (TypeScript `MAX_TIMER_DELAY_MS`).
pub const MAX_TIMER_DELAY_MS: u64 = 2_147_483_647;

const GENERATION_CLOSE_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_ENABLED: bool = true;
const DEFAULT_INITIAL_DELAY_MS: u64 = 500;
const DEFAULT_MAX_DELAY_MS: u64 = 30_000;
const DEFAULT_MAX_ATTEMPTS: u64 = 10;
const RECONNECT_KEYS: &[&str] = &["enabled", "initialDelayMs", "maxDelayMs", "maxAttempts"];

/// Fully resolved reconnect policy captured at plugin load.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedReconnectPolicy {
    /// Reconnect automatically after a lost connection.
    pub enabled: bool,
    /// First reconnect delay in milliseconds; doubles per consecutive failed attempt.
    pub initial_delay_ms: u64,
    /// Backoff ceiling in milliseconds; also the uptime after which the attempt budget resets.
    pub max_delay_ms: u64,
    /// Consecutive failed attempts per outage before giving up.
    pub max_attempts: u64,
}

/// Outcome of the plugin's first connection attempt.
pub(crate) struct ConnectionOutcome {
    /// If the initial connection or tool sync failed, the error; otherwise absent.
    pub error: Option<String>,
}

/// Spawn parameters for one supervised stdio MCP server.
pub(crate) struct StdioConnectSpec {
    /// YAML `serverName`.
    pub server_name: String,
    /// Child executable.
    pub command: String,
    /// Arguments after `command`.
    pub args: Vec<String>,
    /// `config.env` overlay.
    pub env: Vec<EnvEntry>,
    /// Child working directory; empty inherits.
    pub cwd: String,
    /// YAML `toolCallTimeoutMs`.
    pub tool_call_timeout_ms: u64,
}

/// Stops the supervisor and tears down the live generation.
pub(crate) struct ConnectionDisposer {
    shared: Arc<Shared>,
    supervisor: tokio::task::JoinHandle<()>,
}

struct Shared {
    label: String,
    spec: StdioConnectSpec,
    policy: ResolvedReconnectPolicy,
    tools: Arc<Mutex<ToolRuntime>>,
    owned_names: Arc<Mutex<Vec<String>>>,
    child: Arc<AsyncMutex<Option<Child>>>,
    disposed: AtomicBool,
    cancel: Notify,
    failed_attempts: Mutex<u64>,
    connected_at: Mutex<Option<Instant>>,
    first_error: Mutex<Option<String>>,
    ready_tx: Mutex<Option<oneshot::Sender<ConnectionOutcome>>>,
}

enum Attempt {
    Up,
    Failed,
    Stop,
}

/// Resolve raw `reconnect` JSON to the policy the supervisor runs.
///
/// Omission uses the TypeScript defaults (`enabled` true, `initialDelayMs` 500,
/// `maxDelayMs` 30000, `maxAttempts` 10). Unknown keys and inverted delays fail
/// at load before spawn.
///
/// # Parameters
///
/// * `config` - Raw `reconnect` object, or `None` for defaults.
/// * `path` - Diagnostic prefix naming the config location in thrown messages.
///
/// # Returns
///
/// The resolved policy.
///
/// # Errors
///
/// A string using the TypeScript `resolveReconnectPolicy` sentences with `path`.
pub fn resolve_reconnect_policy(
    config: Option<&Value>,
    path: &str,
) -> Result<ResolvedReconnectPolicy, String> {
    if let Some(value) = config {
        let Some(map) = value.as_object() else {
            return Err("mcp-client: reconnect must be an object".into());
        };
        for key in map.keys() {
            if !RECONNECT_KEYS.contains(&key.as_str()) {
                return Err(format!("{path}.{key} is not a reconnect option"));
            }
        }
    }
    let enabled = match config.and_then(|value| value.get("enabled")) {
        None => DEFAULT_ENABLED,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(format!("{path}.enabled must be a boolean"));
        }
    };
    let initial_delay_ms = match config.and_then(|value| value.get("initialDelayMs")) {
        None => DEFAULT_INITIAL_DELAY_MS,
        Some(value) => parse_delay_ms(value, path, "initialDelayMs")?,
    };
    let max_delay_ms = match config.and_then(|value| value.get("maxDelayMs")) {
        None => DEFAULT_MAX_DELAY_MS,
        Some(value) => parse_delay_ms(value, path, "maxDelayMs")?,
    };
    let max_attempts = match config.and_then(|value| value.get("maxAttempts")) {
        None => DEFAULT_MAX_ATTEMPTS,
        Some(value) => parse_max_attempts(value, path)?,
    };
    if initial_delay_ms > max_delay_ms {
        return Err(format!(
            "{path}.initialDelayMs must be less than or equal to maxDelayMs"
        ));
    }
    Ok(ResolvedReconnectPolicy {
        enabled,
        initial_delay_ms,
        max_delay_ms,
        max_attempts,
    })
}

/// Start the supervised stdio connection and keep it alive per `policy`.
///
/// The caller awaits [`ConnectionOutcome`] for startup-await semantics, then
/// [`ConnectionDisposer::dispose`] from `ctx.effect`.
///
/// # Parameters
///
/// * `spec` - Spawn identity, command, env, and per-call timeout.
/// * `policy` - Resolved reconnect policy from [`resolve_reconnect_policy`].
/// * `tools` - Injected tool runtime that receives `mcp__` public names.
/// * `owned_names` - Public names this instance currently owns.
/// * `child` - Slot holding the live stdio child for this generation.
///
/// # Returns
///
/// A `ready` receiver that settles after the first connect attempt, and a
/// disposer that cancels backoff, kills the child, and unregisters names.
pub(crate) fn start_connection(
    spec: StdioConnectSpec,
    policy: ResolvedReconnectPolicy,
    tools: Arc<Mutex<ToolRuntime>>,
    owned_names: Arc<Mutex<Vec<String>>>,
    child: Arc<AsyncMutex<Option<Child>>>,
) -> (oneshot::Receiver<ConnectionOutcome>, ConnectionDisposer) {
    let (ready_tx, ready_rx) = oneshot::channel();
    let label = format!("mcp-client({})", spec.server_name);
    let shared = Arc::new(Shared {
        label,
        spec,
        policy,
        tools,
        owned_names,
        child,
        disposed: AtomicBool::new(false),
        cancel: Notify::new(),
        failed_attempts: Mutex::new(0),
        connected_at: Mutex::new(None),
        first_error: Mutex::new(None),
        ready_tx: Mutex::new(Some(ready_tx)),
    });
    let supervisor = tokio::spawn(run_supervisor(Arc::clone(&shared)));
    (ready_rx, ConnectionDisposer { shared, supervisor })
}

impl ConnectionDisposer {
    /// Stop reconnection, kill the live child, wait up to 5000ms, unregister names.
    ///
    /// # Parameters
    ///
    /// * `self` - Handle that owns the supervisor task and shared generation state.
    ///
    /// # Returns
    ///
    /// Nothing. The supervisor is aborted and owned public names are unregistered.
    pub(crate) async fn dispose(self) {
        self.shared.disposed.store(true, Ordering::SeqCst);
        self.shared.cancel.notify_waiters();
        {
            let mut slot = self.shared.child.lock().await;
            if let Some(mut child) = slot.take() {
                let _ = child.start_kill();
                if tokio::time::timeout(
                    Duration::from_millis(GENERATION_CLOSE_TIMEOUT_MS),
                    child.wait(),
                )
                .await
                .is_err()
                {
                    eprintln!(
                        "{}: generation did not close within {GENERATION_CLOSE_TIMEOUT_MS}ms during disposal — server shutdown may be incomplete",
                        self.shared.label
                    );
                }
            }
        }
        let abort = self.supervisor.abort_handle();
        if tokio::time::timeout(
            Duration::from_millis(GENERATION_CLOSE_TIMEOUT_MS),
            self.supervisor,
        )
        .await
        .is_err()
        {
            abort.abort();
        }
        unregister_owned(&self.shared);
    }
}

async fn run_supervisor(shared: Arc<Shared>) {
    let mut startup = true;
    loop {
        if shared.disposed.load(Ordering::SeqCst) {
            return;
        }
        let attempt = connect_generation(&shared).await;
        if startup {
            finish_ready(&shared, matches!(attempt, Attempt::Up));
            startup = false;
        }
        if shared.disposed.load(Ordering::SeqCst) {
            return;
        }
        match attempt {
            Attempt::Up => {
                wait_for_generation_down(&shared).await;
                if shared.disposed.load(Ordering::SeqCst) {
                    return;
                }
            }
            Attempt::Stop => return,
            Attempt::Failed => {}
        }
        let Some(delay_ms) = schedule_reconnect(&shared) else {
            return;
        };
        let notified = shared.cancel.notified();
        if shared.disposed.load(Ordering::SeqCst) {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
            _ = notified => return,
        }
    }
}

async fn connect_generation(shared: &Shared) -> Attempt {
    if shared.disposed.load(Ordering::SeqCst) {
        return Attempt::Failed;
    }
    let cwd = if shared.spec.cwd.is_empty() {
        None
    } else {
        Some(shared.spec.cwd.as_str())
    };
    let spawned = spawn_stdio(
        &shared.spec.command,
        &shared.spec.args,
        &shared.spec.env,
        cwd,
    );
    let (mut session, child) = match spawned {
        Ok(pair) => pair,
        Err(err) => {
            record_first_error(shared, &err);
            if is_live(shared) {
                eprintln!("{}: connection attempt failed: {err}", shared.label);
            }
            return Attempt::Failed;
        }
    };
    {
        let mut slot = shared.child.lock().await;
        if shared.disposed.load(Ordering::SeqCst) {
            drop(child);
            return Attempt::Failed;
        }
        *slot = Some(child);
    }
    session.set_tool_call_timeout_ms(shared.spec.tool_call_timeout_ms);
    if let Err(err) = session.initialize().await {
        return fail_attempt(shared, err).await;
    }
    let drafts = match session.list_tools().await {
        Ok(drafts) => drafts,
        Err(err) => return fail_attempt(shared, err).await,
    };
    let previous = shared
        .owned_names
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let swapped = {
        let mut runtime = shared.tools.lock().unwrap_or_else(PoisonError::into_inner);
        swap_generation(
            &session,
            &mut runtime,
            &shared.spec.server_name,
            previous,
            drafts,
        )
    };
    match swapped {
        Ok(names) => {
            *shared
                .owned_names
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = names;
        }
        Err(err) => return fail_attempt(shared, err).await,
    }
    if shared.disposed.load(Ordering::SeqCst) {
        return Attempt::Failed;
    }
    *shared
        .connected_at
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    let failed = *shared
        .failed_attempts
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if failed > 0 {
        eprintln!(
            "{}: reconnected and re-synced tools (attempt {failed}/{})",
            shared.label, shared.policy.max_attempts
        );
    }
    Attempt::Up
}

async fn fail_attempt(shared: &Shared, err: impl std::fmt::Display) -> Attempt {
    record_first_error(shared, &err);
    if is_live(shared) {
        eprintln!("{}: connection attempt failed: {err}", shared.label);
    }
    if !quiesce_child(shared).await {
        return Attempt::Stop;
    }
    if !is_live(shared) {
        return Attempt::Failed;
    }
    Attempt::Failed
}

async fn wait_for_generation_down(shared: &Shared) {
    let mut child = {
        let mut slot = shared.child.lock().await;
        match slot.take() {
            Some(child) => child,
            None => return,
        }
    };
    let notified = shared.cancel.notified();
    if shared.disposed.load(Ordering::SeqCst) {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(
            Duration::from_millis(GENERATION_CLOSE_TIMEOUT_MS),
            child.wait(),
        )
        .await;
        return;
    }
    tokio::select! {
        _ = child.wait() => {}
        _ = notified => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(
                Duration::from_millis(GENERATION_CLOSE_TIMEOUT_MS),
                child.wait(),
            )
            .await;
        }
    }
}

async fn quiesce_child(shared: &Shared) -> bool {
    let mut slot = shared.child.lock().await;
    let Some(mut child) = slot.take() else {
        return true;
    };
    let _ = child.start_kill();
    match tokio::time::timeout(
        Duration::from_millis(GENERATION_CLOSE_TIMEOUT_MS),
        child.wait(),
    )
    .await
    {
        Ok(_) => true,
        Err(_) => {
            eprintln!(
                "{}: failed generation did not close within {GENERATION_CLOSE_TIMEOUT_MS}ms — reconnect stopped to avoid overlapping server processes; reload the plugin or restart the Host to retry",
                shared.label
            );
            false
        }
    }
}

fn schedule_reconnect(shared: &Shared) -> Option<u64> {
    if shared.disposed.load(Ordering::SeqCst) {
        return None;
    }
    let lost_established_connection = shared
        .connected_at
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some();
    if !shared.policy.enabled {
        let message = if lost_established_connection {
            "connection lost and reconnect is disabled — registered tools will fail until an HMR reload or Host restart"
        } else {
            "connection failed and reconnect is disabled — no tools were registered; reload the plugin or restart the Host to connect"
        };
        eprintln!("{}: {message}", shared.label);
        return None;
    }
    let mut connected_at = shared
        .connected_at
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut failed_attempts = shared
        .failed_attempts
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = *connected_at {
        if at.elapsed() >= Duration::from_millis(shared.policy.max_delay_ms) {
            *failed_attempts = 0;
        }
    }
    *connected_at = None;
    *failed_attempts += 1;
    if *failed_attempts > shared.policy.max_attempts {
        drop(failed_attempts);
        drop(connected_at);
        unregister_owned(shared);
        eprintln!(
            "{}: giving up after {} consecutive failed reconnect attempts — tools unregistered; reload the plugin or restart the Host to reconnect",
            shared.label, shared.policy.max_attempts
        );
        return None;
    }
    let delay_ms = reconnect_delay_ms(&shared.policy, *failed_attempts);
    let action = if lost_established_connection {
        "connection lost; reconnecting"
    } else {
        "connection failed; retrying"
    };
    eprintln!(
        "{}: {action} in {delay_ms}ms (attempt {}/{})",
        shared.label, *failed_attempts, shared.policy.max_attempts
    );
    Some(delay_ms)
}

fn reconnect_delay_ms(policy: &ResolvedReconnectPolicy, failed_attempts: u64) -> u64 {
    let exp = failed_attempts.saturating_sub(1);
    let backoff = (policy.initial_delay_ms as f64) * 2f64.powi(exp as i32);
    backoff.min(policy.max_delay_ms as f64) as u64
}

fn finish_ready(shared: &Shared, connected: bool) {
    let Some(tx) = shared
        .ready_tx
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    else {
        return;
    };
    let error = if connected {
        None
    } else {
        Some(
            shared
                .first_error
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .unwrap_or_else(|| format!("{}: initial connection failed", shared.label)),
        )
    };
    let _ = tx.send(ConnectionOutcome { error });
}

fn record_first_error(shared: &Shared, err: &impl std::fmt::Display) {
    let mut first = shared
        .first_error
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if first.is_none() {
        *first = Some(err.to_string());
    }
}

fn is_live(shared: &Shared) -> bool {
    !shared.disposed.load(Ordering::SeqCst)
}

fn unregister_owned(shared: &Shared) {
    let names = {
        let mut owned = shared
            .owned_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *owned)
    };
    let mut runtime = shared.tools.lock().unwrap_or_else(PoisonError::into_inner);
    for name in names {
        runtime.unregister(&name);
    }
}

fn parse_delay_ms(value: &Value, path: &str, field: &str) -> Result<u64, String> {
    let err = || {
        format!(
            "{path}.{field} must be a positive finite number no greater than {MAX_TIMER_DELAY_MS}"
        )
    };
    let Some(number) = value.as_f64() else {
        return Err(err());
    };
    if !number.is_finite() || number <= 0.0 || number > MAX_TIMER_DELAY_MS as f64 {
        return Err(err());
    }
    Ok(number as u64)
}

fn parse_max_attempts(value: &Value, path: &str) -> Result<u64, String> {
    let err = || format!("{path}.maxAttempts must be a positive integer");
    let Some(number) = value.as_number() else {
        return Err(err());
    };
    if let Some(value) = number.as_u64() {
        if value >= 1 {
            return Ok(value);
        }
        return Err(err());
    }
    if let Some(value) = number.as_i64() {
        if value >= 1 {
            return Ok(value as u64);
        }
        return Err(err());
    }
    match number.as_f64() {
        Some(value) if value.is_finite() && value >= 1.0 && value.fract() == 0.0 => {
            Ok(value as u64)
        }
        _ => Err(err()),
    }
}
