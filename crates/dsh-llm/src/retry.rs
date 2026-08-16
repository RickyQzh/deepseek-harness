//! Provider-routed model-request retry on `agent/request-error`.

use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use dsh_kernel::{Context, Next};
use dsh_session::{LlmFailure, SessionEvent};
use dsh_tools::AbortFlag;
use serde_json::{Value, json};

/// Kernel waterfall name for model-request error recovery.
pub const EVENT_AGENT_REQUEST_ERROR: &str = "agent/request-error";

/// Recovery choice after a terminal model-request error or abort finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestErrorAction {
    /// Repeat `build_request` and the adapter stream.
    Retry,
    /// End the turn as an error.
    Fail,
}

/// Bounded exponential backoff with symmetric jitter.
#[derive(Clone, Debug, PartialEq)]
pub struct RetryBackoff {
    /// Initial local delay in milliseconds.
    pub initial_delay_ms: u64,
    /// Cap for local and accepted provider delays in milliseconds.
    pub max_delay_ms: u64,
    /// Symmetric random multiplier range around one.
    pub jitter_ratio: f64,
}

/// Provider-owned retry mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryMode {
    /// Retry only configured transient failure codes, up to `max_retries`.
    Normal,
    /// Retry every model-request failure until success, cancellation, or disposal.
    Always,
}

/// Fully resolved provider retry policy.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRetryPolicy {
    /// Normal (bounded) or always (unbounded) retry.
    pub mode: RetryMode,
    /// Maximum eligible retries after the first request (normal mode).
    pub max_retries: u32,
    /// Stable failure codes eligible in normal mode.
    pub retryable_codes: Vec<String>,
    /// Local exponential-backoff and jitter.
    pub backoff: RetryBackoff,
}

impl Default for ResolvedRetryPolicy {
    fn default() -> Self {
        Self {
            mode: RetryMode::Normal,
            max_retries: 2,
            retryable_codes: vec!["EMPTY_RESPONSE".into(), "RATE_LIMIT".into()],
            backoff: RetryBackoff {
                initial_delay_ms: 500,
                max_delay_ms: 10_000,
                jitter_ratio: 0.1,
            },
        }
    }
}

/// Canonical JSON array key for one resolved policy (TypeScript `JSON.stringify` match).
#[must_use]
pub fn policy_key(policy: &ResolvedRetryPolicy) -> String {
    match policy.mode {
        RetryMode::Always => serde_json::to_string(&json!([
            "always",
            policy.backoff.initial_delay_ms,
            policy.backoff.max_delay_ms,
            json_number(policy.backoff.jitter_ratio),
        ]))
        .expect("retry policy key"),
        RetryMode::Normal => {
            let mut codes = policy.retryable_codes.clone();
            codes.sort();
            serde_json::to_string(&json!([
                "normal",
                policy.max_retries,
                codes,
                policy.backoff.initial_delay_ms,
                policy.backoff.max_delay_ms,
                json_number(policy.backoff.jitter_ratio),
            ]))
            .expect("retry policy key")
        }
    }
}

fn json_number(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// One durable retry record to append after the waterfall returns.
#[derive(Clone, Debug, PartialEq)]
pub struct RetryAudit {
    /// `llm/retry` or `llm/retry-started`.
    pub event_type: &'static str,
    /// Payload matching TypeScript `LlmRetryEventData` / `LlmRetryStartedEventData`.
    pub data: Value,
}

impl RetryAudit {
    /// Session event to append at `seq` (`time` is `seq` as i64).
    #[must_use]
    pub fn into_session_event(self, seq: u64) -> SessionEvent {
        match self.event_type {
            "llm/retry-started" => SessionEvent::LlmRetryStarted {
                seq,
                time: seq as i64,
                data: self.data,
                ignorable: None,
            },
            _ => SessionEvent::LlmRetry {
                seq,
                time: seq as i64,
                data: self.data,
                ignorable: None,
            },
        }
    }
}

/// Failed-request facts for [`install`] listeners. Task-local so concurrent agents cannot mix.
pub struct RetryScope {
    failure: LlmFailure,
    turn: u64,
    step: u64,
    provider: String,
    policy: ResolvedRetryPolicy,
    prior_retries: Vec<Value>,
    abort: AbortFlag,
    audit: Vec<RetryAudit>,
}

tokio::task_local! {
    static RETRY_SCOPE: Mutex<RetryScope>;
}

impl RetryScope {
    /// Bind one failed request so [`install`] can schedule a retry without changing waterfall `T`.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        failure: LlmFailure,
        turn: u64,
        step: u64,
        provider: impl Into<String>,
        policy: ResolvedRetryPolicy,
        prior_retries: Vec<Value>,
        abort: AbortFlag,
    ) -> Self {
        Self {
            failure,
            turn,
            step,
            provider: provider.into(),
            policy,
            prior_retries,
            abort,
            audit: Vec::new(),
        }
    }

    /// Run `fut` with this scope as the task-local retry context and return drained audit records.
    pub async fn run<F, R>(self, fut: F) -> (R, Vec<RetryAudit>)
    where
        F: Future<Output = R>,
    {
        RETRY_SCOPE
            .scope(Mutex::new(self), async move {
                let result = fut.await;
                let audit = RETRY_SCOPE
                    .try_with(|slot| std::mem::take(&mut slot.lock().expect("retry scope").audit))
                    .unwrap_or_default();
                (result, audit)
            })
            .await
    }
}

struct Schedule {
    retry_id: String,
    retry: u32,
    delay_ms: u64,
    max_retries: Option<u32>,
    mode: &'static str,
    policy_key: String,
    failure: LlmFailure,
    abort: AbortFlag,
    turn: u64,
    step: u64,
    provider: String,
}

enum RetryPlan {
    Delegate,
    AfterDownstream(Schedule),
    Immediate(Schedule),
}

/// Register the `agent/request-error` waterfall that executes [`RetryScope`] policy.
pub fn install(ctx: &Context) {
    let _ = ctx.on_waterfall(EVENT_AGENT_REQUEST_ERROR, |action, next| async move {
        recover(action, next).await
    });
}

async fn recover(action: RequestErrorAction, next: Next<RequestErrorAction>) -> RequestErrorAction {
    let plan = match RETRY_SCOPE.try_with(|slot| plan_retry(&slot.lock().expect("retry scope"))) {
        Ok(plan) => plan,
        Err(_) => return next(action).await,
    };
    match plan {
        RetryPlan::Delegate => next(action).await,
        RetryPlan::AfterDownstream(schedule) => {
            let downstream = next(action).await;
            if schedule.abort.is_aborted() {
                return RequestErrorAction::Fail;
            }
            if matches!(downstream, RequestErrorAction::Retry) {
                return downstream;
            }
            backoff(schedule).await
        }
        RetryPlan::Immediate(schedule) => backoff(schedule).await,
    }
}

fn plan_retry(scope: &RetryScope) -> RetryPlan {
    match scope.policy.mode {
        RetryMode::Always => match make_schedule(scope) {
            Some(schedule) => RetryPlan::AfterDownstream(schedule),
            None => RetryPlan::Delegate,
        },
        RetryMode::Normal => {
            if !scope
                .policy
                .retryable_codes
                .iter()
                .any(|code| code == &scope.failure.code)
            {
                return RetryPlan::Delegate;
            }
            match make_schedule(scope) {
                Some(schedule) => RetryPlan::Immediate(schedule),
                None => RetryPlan::Delegate,
            }
        }
    }
}

fn make_schedule(scope: &RetryScope) -> Option<Schedule> {
    let key = policy_key(&scope.policy);
    let prior = previous_retry(
        &scope.prior_retries,
        scope.turn,
        scope.step,
        &scope.provider,
        &key,
    );
    let previous = match &prior {
        Some((_, retry)) => *retry,
        None => 0,
    };
    if matches!(scope.policy.mode, RetryMode::Normal) && previous >= scope.policy.max_retries {
        return None;
    }
    let retry = previous + 1;
    let retry_id = match prior {
        Some((id, _)) => id,
        None => new_retry_id(),
    };
    let delay_ms = delay_for(&scope.policy, retry, &scope.failure)?;
    let max_retries = match scope.policy.mode {
        RetryMode::Normal => Some(scope.policy.max_retries),
        RetryMode::Always => None,
    };
    Some(Schedule {
        retry_id,
        retry,
        delay_ms,
        max_retries,
        mode: mode_name(scope.policy.mode),
        policy_key: key,
        failure: scope.failure.clone(),
        abort: scope.abort.clone(),
        turn: scope.turn,
        step: scope.step,
        provider: scope.provider.clone(),
    })
}

fn mode_name(mode: RetryMode) -> &'static str {
    match mode {
        RetryMode::Normal => "normal",
        RetryMode::Always => "always",
    }
}

fn delay_for(policy: &ResolvedRetryPolicy, retry: u32, failure: &LlmFailure) -> Option<u64> {
    match failure.provider_retry_after_ms {
        Some(after) if after > 0 => {
            if after > policy.backoff.max_delay_ms {
                if matches!(policy.mode, RetryMode::Normal) {
                    None
                } else {
                    Some(local_delay(policy, retry, unit_sample()))
                }
            } else {
                Some(after)
            }
        }
        _ => Some(local_delay(policy, retry, unit_sample())),
    }
}

fn local_delay(policy: &ResolvedRetryPolicy, retry: u32, random: f64) -> u64 {
    let exponent = i32::try_from(retry.saturating_sub(1).min(1024)).unwrap_or(1024);
    let initial = policy.backoff.initial_delay_ms as f64;
    let max = policy.backoff.max_delay_ms as f64;
    let exponential = (initial * 2_f64.powi(exponent)).min(max);
    let jitter = 1.0 - policy.backoff.jitter_ratio + 2.0 * policy.backoff.jitter_ratio * random;
    let delay = (exponential * jitter).min(max);
    if delay.is_finite() && delay > 0.0 {
        delay.round() as u64
    } else {
        0
    }
}

fn unit_sample() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
    let mixed = STATE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    (mixed >> 11) as f64 / ((1_u64 << 53) as f64)
}

fn new_retry_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "00000000-0000-4000-8000-{:012x}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn previous_retry(
    prior: &[Value],
    turn: u64,
    step: u64,
    provider: &str,
    policy_key: &str,
) -> Option<(String, u32)> {
    for data in prior.iter().rev() {
        if data.get("turn").and_then(Value::as_u64) != Some(turn) {
            continue;
        }
        if data.get("step").and_then(Value::as_u64) != Some(step) {
            continue;
        }
        let Some(event_provider) = data.get("provider").and_then(Value::as_str) else {
            continue;
        };
        if event_provider != provider {
            continue;
        }
        let Some(event_key) = data.get("policyKey").and_then(Value::as_str) else {
            continue;
        };
        if event_key != policy_key {
            continue;
        }
        let Some(retry_id) = data.get("retryId").and_then(Value::as_str) else {
            continue;
        };
        let Some(retry) = data.get("retry").and_then(Value::as_u64) else {
            continue;
        };
        return Some((retry_id.to_string(), retry as u32));
    }
    None
}

async fn backoff(schedule: Schedule) -> RequestErrorAction {
    let started = RetryAudit {
        event_type: "llm/retry-started",
        data: json!({
            "retryId": schedule.retry_id,
            "turn": schedule.turn,
            "step": schedule.step,
            "retry": schedule.retry,
        }),
    };
    let abort = schedule.abort.clone();
    let delay = Duration::from_millis(schedule.delay_ms);
    let retry_event = RetryAudit {
        event_type: "llm/retry",
        data: retry_payload(&schedule),
    };
    let _ = RETRY_SCOPE.try_with(|slot| {
        slot.lock().expect("retry scope").audit.push(retry_event);
    });
    if abort.is_aborted() {
        return RequestErrorAction::Fail;
    }
    tokio::select! {
        () = tokio::time::sleep(delay) => {}
        () = abort.cancelled() => {
            return RequestErrorAction::Fail;
        }
    }
    let _ = RETRY_SCOPE.try_with(|slot| {
        slot.lock().expect("retry scope").audit.push(started);
    });
    RequestErrorAction::Retry
}

fn retry_payload(schedule: &Schedule) -> Value {
    let mut data = json!({
        "retryId": schedule.retry_id,
        "turn": schedule.turn,
        "step": schedule.step,
        "provider": schedule.provider,
        "mode": schedule.mode,
        "policyKey": schedule.policy_key,
        "retry": schedule.retry,
        "delayMs": schedule.delay_ms,
        "failure": schedule.failure,
    });
    if let Some(max) = schedule.max_retries {
        data["maxRetries"] = json!(max);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::{
        EVENT_AGENT_REQUEST_ERROR, RequestErrorAction, ResolvedRetryPolicy, RetryBackoff,
        RetryMode, RetryScope, install, policy_key,
    };
    use dsh_kernel::Context;
    use dsh_session::LlmFailure;
    use dsh_tools::AbortFlag;

    fn snapshot_policy() -> ResolvedRetryPolicy {
        ResolvedRetryPolicy {
            mode: RetryMode::Normal,
            max_retries: 1,
            retryable_codes: vec!["RATE_LIMIT".into()],
            backoff: RetryBackoff {
                initial_delay_ms: 1,
                max_delay_ms: 1,
                jitter_ratio: 0.0,
            },
        }
    }

    #[test]
    fn crate_dsh_llm_retry_must_not_exist() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates directory");
        assert!(!crates.join("dsh-llm-retry").exists());
    }

    #[test]
    fn policy_key_for_snapshot_policy_matches_inspect() {
        assert_eq!(
            policy_key(&snapshot_policy()),
            r#"["normal",1,["RATE_LIMIT"],1,1,0]"#
        );
    }

    #[tokio::test]
    async fn retry_scope_retries_rate_limit_and_audits_llm_retry() {
        let ctx = Context::new();
        install(&ctx);
        let failure = LlmFailure {
            message: "snapshot transient failure".into(),
            code: "RATE_LIMIT".into(),
            status: Some(429),
            provider_retry_after_ms: None,
            request_id: None,
        };
        let scope = RetryScope::new(
            failure,
            1,
            1,
            "deepseek-official",
            snapshot_policy(),
            Vec::new(),
            AbortFlag::new(),
        );
        let (action, audit) = scope
            .run(ctx.waterfall(EVENT_AGENT_REQUEST_ERROR, RequestErrorAction::Fail))
            .await;
        assert_eq!(action, RequestErrorAction::Retry);
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].event_type, "llm/retry");
        assert_eq!(audit[0].data["failure"]["code"], "RATE_LIMIT");
        assert_eq!(audit[0].data["failure"]["status"], 429);
        assert_eq!(audit[0].data["maxRetries"], 1);
        assert_eq!(audit[0].data["delayMs"], 1);
        assert_eq!(audit[1].event_type, "llm/retry-started");
    }
}
