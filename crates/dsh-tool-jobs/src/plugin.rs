//! Kernel plugin `@deepseek-ai/dsh-tool-jobs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use dsh_agent::{AgentHandle, AgentRegistry};
use dsh_agent_loop::{AgentStatus, CompactionScope, EVENT_AGENT_PRE_STEP, PreStepDecision};
use dsh_boot::{PLUGIN_TOOL_JOBS, PluginRegistry, PluginSetup};
use dsh_jobs::{JobError, JobId, JobSnapshot, JobStatus, KillResult};
use dsh_jobs_local::LocalJobRegistry;
use dsh_kernel::KernelError;
use dsh_session::{ContentBlock, Message, MessageId, MessageRole, MessageSource};
use dsh_tools::{ToolDefinition, ToolError, ToolExecution, ToolRuntime};
use serde_json::{Value, json};

use crate::status_line;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// How an unreported completion reaches an idle owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionDelivery {
    /// Inject into next-step without opening a turn.
    Quiet,
    /// Open a follow-up turn on an idle owner.
    Wakeup,
}

/// Validated `dsh-tool-jobs` configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolJobsConfig {
    /// Wait duration when `job_output` sets `wait` without `timeout_ms`.
    pub wait_timeout_ms: u64,
    /// Hard cap on any single wait.
    pub max_wait_timeout_ms: u64,
    /// Whether a completion opens a turn on an idle owner.
    pub completion_delivery: CompletionDelivery,
    /// Turns one owner may open by completion wakes before notices degrade to injection.
    pub max_consecutive_wakes: u32,
}

impl Default for ToolJobsConfig {
    fn default() -> Self {
        Self {
            wait_timeout_ms: 30_000,
            max_wait_timeout_ms: 600_000,
            completion_delivery: CompletionDelivery::Wakeup,
            max_consecutive_wakes: 3,
        }
    }
}

/// Register `job_output`, `job_list`, and `job_kill` on `tools`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved = resolve_config(&config).map_err(setup_err)?;
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let jobs = ctx.inject::<LocalJobRegistry>("jobs").await?;
            register_job_tools(
                &mut tools.lock().unwrap_or_else(PoisonError::into_inner),
                Arc::clone(&jobs),
                resolved,
            );
            let agents = ctx.get::<AgentRegistry>("agents");
            if let Some(agents) = agents {
                attach_completion_notices(&ctx, jobs, agents, resolved).map_err(setup_err)?;
            }
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_JOBS, setup);
}

const CONFIG_KEYS: &[&str] = &[
    "waitTimeoutMs",
    "maxWaitTimeoutMs",
    "completionDelivery",
    "maxConsecutiveWakes",
];

fn resolve_config(value: &Value) -> Result<ToolJobsConfig, String> {
    match value {
        Value::Null => Ok(ToolJobsConfig::default()),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("ToolJobsConfig: unknown key \"{key}\""));
                }
            }
            let mut config = ToolJobsConfig::default();
            if let Some(wait) = optional_positive_u64(map.get("waitTimeoutMs"), "waitTimeoutMs")? {
                config.wait_timeout_ms = wait;
            }
            if let Some(cap) =
                optional_positive_u64(map.get("maxWaitTimeoutMs"), "maxWaitTimeoutMs")?
            {
                config.max_wait_timeout_ms = cap;
            }
            if let Some(delivery) = optional_delivery(map.get("completionDelivery"))? {
                config.completion_delivery = delivery;
            }
            if let Some(wakes) =
                optional_positive_u32(map.get("maxConsecutiveWakes"), "maxConsecutiveWakes")?
            {
                config.max_consecutive_wakes = wakes;
            }
            if config.wait_timeout_ms > config.max_wait_timeout_ms {
                return Err(format!(
                    "tool-jobs: waitTimeoutMs ({}) exceeds maxWaitTimeoutMs ({})",
                    config.wait_timeout_ms, config.max_wait_timeout_ms
                ));
            }
            Ok(config)
        }
        _ => Err("ToolJobsConfig: config must be an object".into()),
    }
}

fn optional_delivery(value: Option<&Value>) -> Result<Option<CompletionDelivery>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text == "wakeup" => Ok(Some(CompletionDelivery::Wakeup)),
        Some(Value::String(text)) if text == "quiet" => Ok(Some(CompletionDelivery::Quiet)),
        Some(_) => Err("ToolJobsConfig.completionDelivery must be \"wakeup\" or \"quiet\"".into()),
    }
}

fn optional_positive_u64(value: Option<&Value>, key: &str) -> Result<Option<u64>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(item) => {
            let invalid = || format!("tool-jobs: {key} must be a positive integer");
            let Some(number) = item.as_u64() else {
                return Err(invalid());
            };
            if number == 0 {
                return Err(invalid());
            }
            Ok(Some(number))
        }
    }
}

fn optional_positive_u32(value: Option<&Value>, key: &str) -> Result<Option<u32>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(item) => {
            let invalid = || format!("tool-jobs: {key} must be a whole number of turns");
            let Some(number) = item.as_u64() else {
                return Err(invalid());
            };
            if number == 0 {
                return Err(invalid());
            }
            u32::try_from(number).map(Some).map_err(|_| invalid())
        }
    }
}

fn register_job_tools(
    runtime: &mut ToolRuntime,
    jobs: Arc<LocalJobRegistry>,
    config: ToolJobsConfig,
) {
    runtime.register(job_output_definition(Arc::clone(&jobs), config));
    runtime.register(job_list_definition(Arc::clone(&jobs)));
    runtime.register(job_kill_definition(jobs));
}

fn job_output_definition(jobs: Arc<LocalJobRegistry>, config: ToolJobsConfig) -> ToolDefinition {
    ToolDefinition {
        name: "job_output".into(),
        description:
            "Read a background job. Stream jobs return only output since the previous read; \
final-output jobs return their result after settlement. Every response ends with \
`[status: ...]`. Reads are non-blocking unless `wait: true`, which waits up to the configured cap."
                .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Job id returned by the tool that started the background work."
                },
                "wait": {
                    "type": "boolean",
                    "description": "Block until the job reaches a terminal status or the timeout expires. A timed-out wait returns [status: running] and leaves the job alive."
                },
                "timeout_ms": {
                    "type": "number",
                    "description": "Max wait in milliseconds (only meaningful with wait: true). Defaults to the configured wait timeout; capped by the configured maximum."
                }
            },
            "required": ["id"]
        }),
        execute: Box::new(move |args, exec| {
            let jobs = Arc::clone(&jobs);
            Box::pin(async move { execute_job_output(&jobs, config, args, exec).await })
        }),
        render: Box::new(|_args, value| {
            let text = value.get("text").and_then(Value::as_str).unwrap_or("");
            let job = value.get("job");
            let status = job
                .and_then(|job| job.get("status"))
                .and_then(Value::as_str)
                .and_then(parse_status_token)
                .unwrap_or(JobStatus::Running);
            let detail = job
                .and_then(|job| job.get("detail"))
                .and_then(Value::as_str);
            let body = if text.is_empty() {
                "(no new output)"
            } else {
                text
            };
            let separator = if body.ends_with('\n') { "" } else { "\n" };
            let rendered = format!("{}{}{}", body, separator, status_line(status, detail));
            vec![ContentBlock::Text { text: rendered }]
        }),
        is_concurrency_safe: None,
    }
}

fn job_list_definition(jobs: Arc<LocalJobRegistry>) -> ToolDefinition {
    ToolDefinition {
        name: "job_list".into(),
        description:
            "List your background jobs (running and finished) with their ids, kinds, and statuses."
                .into(),
        parameters: json!({
            "type": "object",
            "properties": {}
        }),
        execute: Box::new(move |_args, exec| {
            let jobs = Arc::clone(&jobs);
            Box::pin(async move {
                let listed = jobs.list(exec.session_id.as_ref());
                Ok(json!(listed.iter().map(public_job).collect::<Vec<_>>()))
            })
        }),
        render: Box::new(|_args, value| {
            let jobs = value.as_array();
            let text = match jobs {
                Some(items) if items.is_empty() => "(no background jobs)".to_string(),
                Some(items) => items
                    .iter()
                    .map(format_list_row)
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => "(no background jobs)".to_string(),
            };
            vec![ContentBlock::Text { text }]
        }),
        is_concurrency_safe: None,
    }
}

fn job_kill_definition(jobs: Arc<LocalJobRegistry>) -> ToolDefinition {
    ToolDefinition {
        name: "job_kill".into(),
        description: "Request cancellation of a running background job by job id. Returns immediately; the job settles as killed once its work actually stops."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Job id returned by the tool that started the background work."
                }
            },
            "required": ["id"]
        }),
        execute: Box::new(move |args, exec| {
            let jobs = Arc::clone(&jobs);
            Box::pin(async move {
                let id = parse_job_id(&args)?;
                let caller = exec.session_id.as_ref();
                let result = jobs
                    .kill(&id, caller, None)
                    .map_err(job_tool_error)?;
                let snapshot = jobs.get(&id, caller).map_err(job_tool_error)?;
                let outcome = match result {
                    KillResult::AlreadyFinished => "already-finished",
                    KillResult::Requested => "cancellation-requested",
                };
                Ok(json!({
                    "outcome": outcome,
                    "job": public_job(&snapshot),
                }))
            })
        }),
        render: Box::new(|_args, value| {
            let outcome = value.get("outcome").and_then(Value::as_str).unwrap_or("");
            let id = value
                .get("job")
                .and_then(|job| job.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let text = if outcome == "already-finished" {
                let job = value.get("job");
                let status = job
                    .and_then(|job| job.get("status"))
                    .and_then(Value::as_str)
                    .and_then(parse_status_token)
                    .unwrap_or(JobStatus::Completed);
                let detail = job
                    .and_then(|job| job.get("detail"))
                    .and_then(Value::as_str);
                format!("job {id} had already finished {}", status_line(status, detail))
            } else {
                format!("requested cancellation of job {id}")
            };
            vec![ContentBlock::Text { text }]
        }),
        is_concurrency_safe: None,
    }
}

async fn execute_job_output(
    jobs: &LocalJobRegistry,
    config: ToolJobsConfig,
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let id = parse_job_id(&args)?;
    let caller = exec.session_id.as_ref();
    if args.get("wait") == Some(&Value::Bool(true)) {
        let requested = args
            .get("timeout_ms")
            .and_then(json_positive_u64)
            .unwrap_or(config.wait_timeout_ms);
        let timeout = requested.min(config.max_wait_timeout_ms);
        tokio::select! {
            result = jobs.wait(&id, timeout, caller) => {
                result.map_err(job_tool_error)?;
            }
            () = exec.signal.cancelled() => {}
        }
    }
    let read = jobs.read(&id, caller).map_err(job_tool_error)?;
    Ok(json!({
        "text": read.text,
        "job": public_job(&read.snapshot),
    }))
}

fn parse_job_id(args: &Value) -> Result<JobId, ToolError> {
    match args.get("id") {
        Some(Value::String(id)) if !id.is_empty() => Ok(JobId::new(id.clone())),
        Some(Value::String(id)) => Err(ToolError::Other(format!(
            "invalid id: expected a non-empty string, got {}",
            json!(id)
        ))),
        _ => Err(ToolError::Other(
            "invalid id: expected a non-empty string".into(),
        )),
    }
}

fn json_positive_u64(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        if number > 0 {
            return Some(number);
        }
        return None;
    }
    let number = value.as_f64()?;
    if number.is_finite() && number > 0.0 {
        return Some(number as u64);
    }
    None
}

fn job_tool_error(error: JobError) -> ToolError {
    ToolError::Other(error.to_string())
}

fn parse_status_token(token: &str) -> Option<JobStatus> {
    match token {
        "running" => Some(JobStatus::Running),
        "stopping" => Some(JobStatus::Stopping),
        "completed" => Some(JobStatus::Completed),
        "killed" => Some(JobStatus::Killed),
        "failed" => Some(JobStatus::Failed),
        _ => None,
    }
}

fn public_job(snapshot: &JobSnapshot) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(snapshot.id().as_str()));
    map.insert("kind".into(), json!(snapshot.kind().as_str()));
    map.insert("label".into(), json!(snapshot.label()));
    map.insert("status".into(), json!(snapshot.status().as_str()));
    if let Some(detail) = snapshot.detail() {
        map.insert("detail".into(), json!(detail));
    }
    map.insert("startedAt".into(), json!(snapshot.started_at()));
    if let Some(finished) = snapshot.finished_at() {
        map.insert("finishedAt".into(), json!(finished));
    }
    Value::Object(map)
}

fn format_list_row(value: &Value) -> String {
    let id = value.get("id").and_then(Value::as_str).unwrap_or("");
    let kind = value.get("kind").and_then(Value::as_str).unwrap_or("");
    let status = value.get("status").and_then(Value::as_str).unwrap_or("");
    let label = value.get("label").and_then(Value::as_str).unwrap_or("");
    format!("{id} [{kind}] {status} — {label}")
}

fn attach_completion_notices(
    ctx: &dsh_kernel::Context,
    jobs: Arc<LocalJobRegistry>,
    agents: Arc<AgentRegistry>,
    config: ToolJobsConfig,
) -> Result<(), String> {
    let spent_wakes = Arc::new(Mutex::new(HashMap::<String, u32>::new()));
    if config.completion_delivery == CompletionDelivery::Wakeup {
        let spent = Arc::clone(&spent_wakes);
        ctx.on_waterfall::<PreStepDecision, _, _>(EVENT_AGENT_PRE_STEP, move |decision, next| {
            let spent = Arc::clone(&spent);
            async move {
                reset_wake_budget_on_user_input(&decision, &spent);
                next(decision).await
            }
        })
        .map_err(|error| error.to_string())?;
    }
    jobs.on_job_done(move |snapshot| {
        deliver_completion(&agents, &spent_wakes, config, snapshot);
    });
    Ok(())
}

fn reset_wake_budget_on_user_input(
    decision: &PreStepDecision,
    spent: &Mutex<HashMap<String, u32>>,
) {
    let PreStepDecision::Enter { messages } = decision else {
        return;
    };
    let has_user = messages
        .iter()
        .any(|message| matches!(message.source, MessageSource::User));
    if !has_user {
        return;
    }
    let Some(session_id) = CompactionScope::try_current(|scope| {
        scope.with_session(|session| session.id().as_str().to_string())
    }) else {
        return;
    };
    spent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&session_id);
}

fn deliver_completion(
    agents: &AgentRegistry,
    spent_wakes: &Arc<Mutex<HashMap<String, u32>>>,
    config: ToolJobsConfig,
    snapshot: JobSnapshot,
) {
    if snapshot.reported() {
        return;
    }
    let Some(owner) = snapshot.owner_session() else {
        return;
    };
    let Some(handle) = agents.get(owner.as_str()) else {
        return;
    };
    let idle = handle.lock().status() == AgentStatus::Idle;
    let key = owner.as_str().to_string();
    let mut spent_guard = spent_wakes.lock().unwrap_or_else(PoisonError::into_inner);
    let spent = *spent_guard.get(&key).unwrap_or(&0);
    let wakeup = config.completion_delivery == CompletionDelivery::Wakeup
        && idle
        && spent < config.max_consecutive_wakes;
    if wakeup {
        spent_guard.insert(key, spent.saturating_add(1));
    }
    drop(spent_guard);
    let message = completion_message(&snapshot);
    tokio::spawn(async move {
        dispatch_notice(handle, message, wakeup).await;
    });
}

async fn dispatch_notice(handle: AgentHandle, message: Message, wakeup: bool) {
    if wakeup {
        let _ = handle.followup(message).await;
    } else {
        let _ = handle.inject(message).await;
    }
}

fn completion_message(snapshot: &JobSnapshot) -> Message {
    let line = status_line(snapshot.status(), snapshot.detail());
    let text = format!(
        "background job {} ({}: {}) finished {}. Read its output with job_output.",
        snapshot.id().as_str(),
        snapshot.kind().as_str(),
        snapshot.label(),
        line
    );
    let summary = format!("{} {} {}", snapshot.kind().as_str(), snapshot.label(), line);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    Message {
        id: MessageId::new(format!("job-notice-{}-{nanos}", std::process::id())),
        role: MessageRole::User,
        content: vec![ContentBlock::Text { text }],
        source: MessageSource::Plugin {
            plugin: "tool-jobs".into(),
            form: Some("notice".into()),
            sections: Vec::new(),
            summary: Some(summary),
            compaction_id: None,
            source_command_id: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::status_line;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_jobs::{JobHooks, JobKind, JobOutcome, JobSnapshot, JobStart, JobStatus};
    use dsh_jobs_local::LocalJobRegistry;
    use dsh_kernel::Context;
    use dsh_session::{CallId, SessionId};
    use dsh_tools::{AbortFlag, ToolExecutionInput, ToolExecutionResult, ToolRuntime};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    async fn boot_jobs_tools() -> (
        Context,
        std::sync::Arc<LocalJobRegistry>,
        std::sync::Arc<Mutex<ToolRuntime>>,
    ) {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_tools::plugin::register(&mut registry);
        dsh_jobs_local::plugin::register(&mut registry);
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-tools'\n- name: '@deepseek-ai/dsh-jobs-local'\n- name: '@deepseek-ai/dsh-tool-jobs'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot tool-jobs stack");
        let jobs = ctx.inject::<LocalJobRegistry>("jobs").await.expect("jobs");
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        (ctx, jobs, tools)
    }

    fn hanging_job() -> JobStart {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let tx = std::sync::Mutex::new(Some(tx));
        JobStart {
            kind: JobKind::Bash,
            label: "hang".into(),
            owner_session: None,
            run: Box::new(move || JobHooks {
                cancel: Box::new(move |_| {
                    if let Some(sender) = tx
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                    {
                        let _ = sender.send(());
                    }
                }),
                done: Box::pin(async move {
                    let _ = rx.await;
                    JobOutcome {
                        status: JobStatus::Killed,
                        detail: None,
                        output: None,
                    }
                }),
                read_output: None,
            }),
        }
    }

    fn execute_input(name: &str, arguments: serde_json::Value) -> ToolExecutionInput {
        execute_input_with_session(name, arguments, None)
    }

    fn execute_input_with_session(
        name: &str,
        arguments: serde_json::Value,
        session_id: Option<SessionId>,
    ) -> ToolExecutionInput {
        ToolExecutionInput {
            call_id: CallId::new("c1"),
            root_call_id: None,
            name: name.into(),
            arguments,
            parent: None,
            session_id,
            signal: AbortFlag::new(),
        }
    }

    fn result_text(result: &ToolExecutionResult) -> String {
        match result.content() {
            [dsh_session::ContentBlock::Text { text }] => text.clone(),
            other => panic!("unexpected content {other:?}"),
        }
    }

    #[tokio::test]
    async fn registers_three_job_tools_without_agents() {
        let (_ctx, _jobs, tools) = boot_jobs_tools().await;
        let mut names = tools.lock().expect("tools").registered_names();
        names.sort();
        assert_eq!(
            names,
            vec![
                "job_kill".to_string(),
                "job_list".to_string(),
                "job_output".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn job_list_empty_renders_no_background_jobs() {
        let (_ctx, _jobs, tools) = boot_jobs_tools().await;
        let result = tools
            .lock()
            .expect("tools")
            .execute(execute_input("job_list", json!({})))
            .await;
        assert!(!result.is_error());
        assert_eq!(result_text(&result), "(no background jobs)");
    }

    #[tokio::test]
    async fn job_output_renders_status_and_empty_id_fails() {
        let (_ctx, jobs, tools) = boot_jobs_tools().await;
        jobs.start(JobStart {
            kind: JobKind::Bash,
            label: "echo".into(),
            owner_session: None,
            run: Box::new(|| JobHooks {
                cancel: Box::new(|_| {}),
                done: Box::pin(std::future::pending::<JobOutcome>()),
                read_output: Some(Box::new(|| "line one\n".into())),
            }),
        })
        .unwrap();
        let result = tools
            .lock()
            .expect("tools")
            .execute(execute_input("job_output", json!({ "id": "bash-1" })))
            .await;
        assert!(!result.is_error());
        assert_eq!(result_text(&result), "line one\n[status: running]");
        let empty = tools
            .lock()
            .expect("tools")
            .execute(execute_input("job_output", json!({ "id": "" })))
            .await;
        assert!(empty.is_error());
        match empty {
            ToolExecutionResult::Failure { error, .. } => {
                assert!(error.message.contains("invalid id"));
            }
            ToolExecutionResult::Success { .. } => panic!("expected failure"),
        }
    }

    #[tokio::test]
    async fn unknown_job_error_contains_unknown_job() {
        let (_ctx, _jobs, tools) = boot_jobs_tools().await;
        let result = tools
            .lock()
            .expect("tools")
            .execute(execute_input("job_output", json!({ "id": "bash-99" })))
            .await;
        assert!(result.is_error());
        match result {
            ToolExecutionResult::Failure { error, .. } => {
                assert!(error.message.contains("unknown job"));
            }
            ToolExecutionResult::Success { .. } => panic!("expected failure"),
        }
    }

    #[tokio::test]
    async fn job_kill_requests_cancellation() {
        let (_ctx, jobs, tools) = boot_jobs_tools().await;
        jobs.start(hanging_job()).unwrap();
        let result = tools
            .lock()
            .expect("tools")
            .execute(execute_input("job_kill", json!({ "id": "bash-1" })))
            .await;
        assert!(!result.is_error());
        assert_eq!(result_text(&result), "requested cancellation of job bash-1");
    }

    #[tokio::test]
    async fn job_output_wait_timeout_is_not_a_tool_error() {
        let (_ctx, jobs, tools) = boot_jobs_tools().await;
        jobs.start(hanging_job()).unwrap();
        let result = tools
            .lock()
            .expect("tools")
            .execute(execute_input(
                "job_output",
                json!({ "id": "bash-1", "wait": true, "timeout_ms": 20 }),
            ))
            .await;
        assert!(!result.is_error());
        assert!(result_text(&result).contains("[status: running]"));
    }

    #[tokio::test]
    async fn wait_timeout_above_cap_fails_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_tools::plugin::register(&mut registry);
        dsh_jobs_local::plugin::register(&mut registry);
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-tools'\n- name: '@deepseek-ai/dsh-jobs-local'\n- name: '@deepseek-ai/dsh-tool-jobs'\n  config:\n    waitTimeoutMs: 100\n    maxWaitTimeoutMs: 50\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("waitTimeoutMs (100) exceeds maxWaitTimeoutMs (50)")
        );
    }

    #[test]
    fn status_line_matches_typescript() {
        assert_eq!(status_line(JobStatus::Running, None), "[status: running]");
    }

    #[tokio::test]
    async fn owned_job_is_visible_to_matching_session_id() {
        let (_ctx, jobs, tools) = boot_jobs_tools().await;
        jobs.start(JobStart {
            kind: JobKind::Bash,
            label: "echo".into(),
            owner_session: Some(SessionId::new("a")),
            run: Box::new(|| JobHooks {
                cancel: Box::new(|_| {}),
                done: Box::pin(std::future::pending::<JobOutcome>()),
                read_output: Some(Box::new(|| "owned\n".into())),
            }),
        })
        .unwrap();
        let list_a = tools
            .lock()
            .expect("tools")
            .execute(execute_input_with_session(
                "job_list",
                json!({}),
                Some(SessionId::new("a")),
            ))
            .await;
        assert!(!list_a.is_error());
        assert!(result_text(&list_a).contains("bash-1"));
        let output_a = tools
            .lock()
            .expect("tools")
            .execute(execute_input_with_session(
                "job_output",
                json!({ "id": "bash-1" }),
                Some(SessionId::new("a")),
            ))
            .await;
        assert!(!output_a.is_error());
        assert!(result_text(&output_a).contains("owned"));
        let list_b = tools
            .lock()
            .expect("tools")
            .execute(execute_input_with_session(
                "job_list",
                json!({}),
                Some(SessionId::new("b")),
            ))
            .await;
        assert!(!list_b.is_error());
        assert_eq!(result_text(&list_b), "(no background jobs)");
        let output_b = tools
            .lock()
            .expect("tools")
            .execute(execute_input_with_session(
                "job_output",
                json!({ "id": "bash-1" }),
                Some(SessionId::new("b")),
            ))
            .await;
        assert!(output_b.is_error());
        match output_b {
            ToolExecutionResult::Failure { error, .. } => {
                assert!(error.message.contains("belongs to another session"));
            }
            ToolExecutionResult::Success { .. } => panic!("expected foreign get to fail"),
        }
    }

    #[tokio::test]
    async fn aborted_job_output_wait_does_not_suppress_on_job_done() {
        let (_ctx, jobs, tools) = boot_jobs_tools().await;
        let seen = Arc::new(Mutex::new(None::<JobSnapshot>));
        let slot = Arc::clone(&seen);
        jobs.on_job_done(move |snap| {
            *slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(snap);
        });
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        jobs.start(JobStart {
            kind: JobKind::Bash,
            label: "hang".into(),
            owner_session: Some(SessionId::new("a")),
            run: Box::new(move || JobHooks {
                cancel: Box::new(|_| {}),
                done: Box::pin(async move {
                    let _ = rx.await;
                    JobOutcome {
                        status: JobStatus::Completed,
                        detail: None,
                        output: None,
                    }
                }),
                read_output: None,
            }),
        })
        .unwrap();
        let signal = AbortFlag::new();
        let mut input = execute_input_with_session(
            "job_output",
            json!({ "id": "bash-1", "wait": true, "timeout_ms": 60_000 }),
            Some(SessionId::new("a")),
        );
        input.signal = signal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            signal.abort();
        });
        let _ = tools.lock().expect("tools").execute(input).await;
        tx.send(()).expect("settle");
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(snap) = seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                assert!(
                    !snap.reported(),
                    "aborted wait must not mark the job reported"
                );
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("on_job_done did not fire after aborted tool wait");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
