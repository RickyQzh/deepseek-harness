//! Model-facing `subagent` / `subagent_fork` delegation tools.

use std::sync::Arc;

use dsh_agent::{AgentHandle, AgentRegistry};
use dsh_session::ContentBlock;
use dsh_subagent::{
    ContinuableStartSpec, SubagentResult, SubagentRuntime, SubagentStartRequest, SubagentStopReason,
};
use dsh_tools::{ToolDefinition, ToolError, ToolExecution, ToolRuntime};
use serde_json::{Value, json};

use crate::util::setup_err;

/// Background policy for one delegation-tool instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundMode {
    /// Wait for [`SubagentRuntime::start`].
    OneShot,
    /// Default to [`SubagentRuntime::start_continuable_background`].
    Continuable,
}

/// Validated delegation-tool configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegateConfig {
    /// `subagents` provider name.
    pub provider: String,
    /// Model-facing tool name.
    pub tool_name: String,
    /// Whether `run_in_background` is accepted.
    pub enable_run_in_background: bool,
    /// Background lifecycle policy.
    pub background_mode: BackgroundMode,
    /// Optional absolute delegation-depth cap.
    pub max_depth: Option<u32>,
}

const CONFIG_KEYS: &[&str] = &[
    "provider",
    "toolName",
    "enableRunInBackground",
    "backgroundMode",
    "maxDepth",
];

/// Resolve YAML config. `provider` is required.
///
/// # Errors
///
/// Unknown keys, missing provider, or invalid field types.
pub fn resolve_config(value: &Value) -> Result<DelegateConfig, String> {
    crate::util::reject_unknown_keys(value, CONFIG_KEYS, "ToolSubagentConfig")?;
    match value {
        Value::Null => Err("ToolSubagentConfig.provider is required".into()),
        Value::Object(map) => {
            let provider = match map.get("provider") {
                Some(Value::String(name)) if !name.is_empty() => name.clone(),
                Some(_) => {
                    return Err("ToolSubagentConfig.provider must be a non-empty string".into());
                }
                None => return Err("ToolSubagentConfig.provider is required".into()),
            };
            let tool_name = match map.get("toolName") {
                None | Some(Value::Null) => "subagent".to_string(),
                Some(Value::String(name)) if !name.is_empty() => name.clone(),
                Some(_) => {
                    return Err("ToolSubagentConfig.toolName must be a non-empty string".into());
                }
            };
            let enable_run_in_background = match map.get("enableRunInBackground") {
                None | Some(Value::Null) => true,
                Some(Value::Bool(flag)) => *flag,
                Some(_) => {
                    return Err("ToolSubagentConfig.enableRunInBackground must be a boolean".into());
                }
            };
            let background_mode = match map.get("backgroundMode") {
                None | Some(Value::Null) => BackgroundMode::OneShot,
                Some(Value::String(mode)) if mode == "one-shot" => BackgroundMode::OneShot,
                Some(Value::String(mode)) if mode == "continuable" => BackgroundMode::Continuable,
                Some(_) => {
                    return Err(
                        "ToolSubagentConfig.backgroundMode must be \"one-shot\" or \"continuable\""
                            .into(),
                    );
                }
            };
            let max_depth = match map.get("maxDepth") {
                None | Some(Value::Null) => Some(3),
                Some(Value::String(text)) if text == "provider-managed" => None,
                Some(value) => match value.as_u64() {
                    Some(number) if number <= u64::from(u32::MAX) => Some(number as u32),
                    _ => {
                        return Err(
                            "ToolSubagentConfig.maxDepth must be a non-negative integer or \"provider-managed\""
                                .into(),
                        )
                    }
                },
            };
            Ok(DelegateConfig {
                provider,
                tool_name,
                enable_run_in_background,
                background_mode,
                max_depth,
            })
        }
        _ => Err("ToolSubagentConfig: config must be an object".into()),
    }
}

/// Register one delegation tool on `tools`.
pub fn register_delegate_tool(
    tools: &mut ToolRuntime,
    subagents: Arc<SubagentRuntime>,
    agents: Arc<AgentRegistry>,
    config: DelegateConfig,
) -> Result<(), dsh_kernel::KernelError> {
    let selected = subagents.get_provider(&config.provider).ok_or_else(|| {
        setup_err(format!(
            "tool-subagent: unknown provider \"{}\"",
            config.provider
        ))
    })?;
    if config.max_depth.is_some() && !selected.capabilities().depth_limit {
        return Err(setup_err(format!(
            "tool-subagent: provider \"{}\" cannot enforce maxDepth (no depthLimit capability)",
            config.provider
        )));
    }
    tools.register(delegate_definition(
        subagents,
        agents,
        config,
        selected.inherits_parent_context(),
    ));
    Ok(())
}

fn delegate_definition(
    subagents: Arc<SubagentRuntime>,
    agents: Arc<AgentRegistry>,
    config: DelegateConfig,
    inherits: bool,
) -> ToolDefinition {
    let wording = provider_wording(inherits);
    let continuable = config.background_mode == BackgroundMode::Continuable;
    let background_enabled = config.enable_run_in_background;
    let description = format!(
        "{}{}",
        wording.description,
        background_suffix(background_enabled, continuable)
    );
    let run_in_background_desc = if continuable {
        "Whether to run in the background and return a durable subagent id immediately. Defaults to true. Set false to wait for the result when your next action depends on it."
    } else {
        "Whether to run as a background job and return its id. Defaults to false; collect with job_output or stop with job_kill."
    };
    let mut properties = json!({
        "description": {
            "type": "string",
            "description": "A short (3-5 word) description of the delegated task, for display."
        },
        "prompt": {
            "type": "string",
            "description": wording.prompt_description
        }
    });
    if background_enabled {
        properties.as_object_mut().expect("object").insert(
            "run_in_background".into(),
            json!({
                "type": "boolean",
                "description": run_in_background_desc
            }),
        );
    }
    ToolDefinition {
        name: config.tool_name.clone(),
        description,
        parameters: json!({
            "type": "object",
            "properties": properties,
            "required": ["description", "prompt"]
        }),
        execute: Box::new(move |args, exec| {
            let subagents = Arc::clone(&subagents);
            let agents = Arc::clone(&agents);
            let config = config.clone();
            Box::pin(
                async move { execute_delegate(&subagents, &agents, &config, args, exec).await },
            )
        }),
        render: Box::new(|_args, value| {
            let text = match value.get("kind").and_then(Value::as_str) {
                Some("continuable") => {
                    let id = value
                        .get("subagentId")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    format!("started subagent {id}")
                }
                Some("foreground") => output_value_text(value.get("output")),
                Some("background") => {
                    let id = value.get("jobId").and_then(Value::as_str).unwrap_or("");
                    format!("started background subagent task {id}")
                }
                _ => String::new(),
            };
            vec![ContentBlock::Text { text }]
        }),
        is_concurrency_safe: Some(Box::new(|_| true)),
    }
}

struct ProviderWording {
    description: &'static str,
    prompt_description: &'static str,
}

fn provider_wording(inherits: bool) -> ProviderWording {
    if inherits {
        ProviderWording {
            description: "Delegate a task to a subagent that inherits this conversation: a child agent seeded with all \
                 completed turns so far (it does not see the current in-flight turn). Use this when the subtask \
                 builds on this conversation's context — a follow-up analysis, \
                 a review, a continuation — without consuming this conversation's context for the work itself. \
                 You receive its result, not its intermediate steps.",
            prompt_description: "The task for the subagent. It already sees this conversation's completed turns, so build on them \
                 freely and state only what is new.",
        }
    } else {
        ProviderWording {
            description: "Delegate a self-contained task to a subagent (a separate agent that works in its own context) \
                 to offload focused, independent work — research, a scoped \
                 implementation, an analysis — so it does not consume this conversation's context. The subagent \
                 returns its result, not its intermediate steps. Give it a \
                 complete, standalone prompt: it does not see this conversation.",
            prompt_description: "The complete, self-contained task for the subagent. It does not share this \
                 conversation's context, so include everything it needs.",
        }
    }
}

fn background_suffix(enabled: bool, continuable: bool) -> &'static str {
    if !enabled {
        return " This call waits for the subagent and returns its result.";
    }
    if continuable {
        " This tool runs in the background by default, immediately returns a durable subagent id, and keeps the child conversation available for later turns. When that run settles, the runtime sends the parent a notice containing its outcome and any final assistant message; `send_message` starts a later turn in the same child conversation. Set `run_in_background: false` only when your next action depends on receiving the result."
    } else {
        " This call waits for the result by default. Set `run_in_background: true` to return a job id; collect with `job_output` and stop with `job_kill`."
    }
}

async fn execute_delegate(
    subagents: &SubagentRuntime,
    agents: &AgentRegistry,
    config: &DelegateConfig,
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let parent = parent_handle(agents, &exec)?;
    let description = required_string(&args, "description")?;
    let prompt = required_string(&args, "prompt")?;
    let continuable = config.background_mode == BackgroundMode::Continuable;
    let run_in_background = resolve_run_in_background(&args, config, continuable)?;
    if continuable && run_in_background {
        let child_id = subagents
            .start_continuable_background(ContinuableStartSpec {
                provider: config.provider.clone(),
                label: description,
                prompt: vec![ContentBlock::Text { text: prompt }],
                parent,
                signal: exec.signal,
            })
            .await
            .map_err(|error| ToolError::Other(error.to_string()))?;
        return Ok(json!({
            "kind": "continuable",
            "subagentId": child_id.as_str(),
        }));
    }
    if continuable {
        let child_id = subagents
            .start_continuable(ContinuableStartSpec {
                provider: config.provider.clone(),
                label: description,
                prompt: vec![ContentBlock::Text { text: prompt }],
                parent,
                signal: exec.signal,
            })
            .await
            .map_err(|error| ToolError::Other(error.to_string()))?;
        return Ok(json!({
            "kind": "continuable",
            "subagentId": child_id.as_str(),
        }));
    }
    if run_in_background {
        return Err(ToolError::Other(
            "background jobs unavailable: one-shot background starts require a jobs service".into(),
        ));
    }
    let result = subagents
        .start(
            &config.provider,
            SubagentStartRequest {
                label: Some(description),
                prompt: vec![ContentBlock::Text { text: prompt }],
                parent_id: parent.id().clone(),
                signal: exec.signal,
                max_depth: config.max_depth,
            },
            parent,
        )
        .await
        .map_err(|error| ToolError::Other(error.to_string()))?;
    foreground_result(result)
}

fn parent_handle(agents: &AgentRegistry, exec: &ToolExecution) -> Result<AgentHandle, ToolError> {
    let Some(session_id) = exec.session_id.as_ref() else {
        return Err(ToolError::Other(
            "subagent tool requires a calling agent (exec.session_id was None)".into(),
        ));
    };
    agents.get(session_id.as_str()).ok_or_else(|| {
        ToolError::Other(format!(
            "subagent tool requires a calling agent (unknown session {})",
            session_id.as_str()
        ))
    })
}

fn resolve_run_in_background(
    args: &Value,
    config: &DelegateConfig,
    continuable: bool,
) -> Result<bool, ToolError> {
    match args.get("run_in_background") {
        None | Some(Value::Null) => Ok(if config.enable_run_in_background {
            continuable
        } else {
            false
        }),
        Some(Value::Bool(true)) => {
            if config.enable_run_in_background {
                Ok(true)
            } else {
                Err(ToolError::Other(
                    "run_in_background is disabled for this tool instance (enableRunInBackground: false)"
                        .into(),
                ))
            }
        }
        Some(Value::Bool(false)) => Ok(false),
        Some(_) => Err(ToolError::Other(
            "run_in_background must be a boolean".into(),
        )),
    }
}

fn required_string(args: &Value, key: &str) -> Result<String, ToolError> {
    match args.get(key).and_then(Value::as_str) {
        Some(text) if !text.is_empty() => Ok(text.to_string()),
        _ => Err(ToolError::Other(format!("{key} is required"))),
    }
}

fn foreground_result(result: SubagentResult) -> Result<Value, ToolError> {
    if let Some(error) = stop_reason_error(result.stop_reason) {
        let text = with_partial_text(&error, &result.output);
        return Err(ToolError::Other(text));
    }
    Ok(json!({
        "kind": "foreground",
        "runId": "one-shot",
        "output": result.output.iter().filter_map(|block| match block {
            ContentBlock::Text { text } => Some(json!({"type": "text", "text": text})),
            _ => None,
        }).collect::<Vec<_>>(),
    }))
}

fn stop_reason_error(reason: SubagentStopReason) -> Option<String> {
    match reason {
        SubagentStopReason::Completed => None,
        SubagentStopReason::Aborted => Some("subagent run was cancelled".into()),
        SubagentStopReason::Error => Some("subagent run failed".into()),
        SubagentStopReason::MaxTokens => {
            Some("subagent run hit its token limit before finishing".into())
        }
        SubagentStopReason::Refusal => Some("subagent declined the task".into()),
    }
}

fn with_partial_text(error: &str, output: &[ContentBlock]) -> String {
    let text = output
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    if text.is_empty() {
        error.to_string()
    } else {
        format!("{error}\nPartial output before the run ended:\n{text}")
    }
}

fn output_value_text(output: Option<&Value>) -> String {
    let Some(Value::Array(items)) = output else {
        return String::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                item.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::register_delegate_tool;
    use crate::register;
    use dsh_agent::{AgentRegistry, CreateAgentOptions};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response};
    use dsh_session::{ContentBlock, Message, MessageId, MessageRole, MessageSource, SessionId};
    use dsh_subagent::{EVENT_SUBAGENT_START, SubagentRuntime, SubagentStartRequest};
    use dsh_subagent_in_process::{register_fork, register_spawn};
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{AbortFlag, ToolPresentationMode, ToolRuntime};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    async fn wait_child_id(slot: &Mutex<Option<SessionId>>) -> SessionId {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(id) = slot.lock().unwrap_or_else(PoisonError::into_inner).clone() {
                return id;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("timed out waiting for subagent/start");
            }
            tokio::task::yield_now().await;
        }
    }

    async fn tool_names_of_fork_child() -> Vec<String> {
        let adapter = Arc::new(MockAdapter::new(vec![
            MockScript::Chunks(text_response("parent-ack")),
            MockScript::Chunks(text_response("child-ok")),
        ]));
        let ctx = Context::new();
        let llm = Arc::new(Mutex::new(LlmRuntime::new()));
        llm.lock().expect("llm").register_adapter("mock", adapter);
        let tools = Arc::new(Mutex::new(ToolRuntime::new(ToolPresentationMode::Native)));
        let prompt = SystemPrompt::new(SystemPromptConfig::default()).unwrap();
        ctx.provide(
            "agents",
            AgentRegistry::from_shared(ctx.clone(), Arc::clone(&llm), Arc::clone(&tools), prompt),
        )
        .expect("provide agents");
        let agents = ctx.inject::<AgentRegistry>("agents").await.expect("agents");
        let last_child = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&last_child);
        ctx.on(EVENT_SUBAGENT_START, move |payload| {
            let info = payload
                .downcast_ref::<dsh_subagent::SubagentRunInfo>()
                .cloned();
            let slot = Arc::clone(&slot);
            async move {
                if let Some(info) = info {
                    *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(info.id);
                }
            }
        })
        .expect("listen");
        let mut registry = PluginRegistry::new();
        dsh_subagent::register(&mut registry);
        register_spawn(&mut registry);
        register_fork(&mut registry);
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-subagent'\n- name: '@deepseek-ai/dsh-subagent-spawn-in-process'\n- name: '@deepseek-ai/dsh-subagent-fork-in-process'\n- name: '@deepseek-ai/dsh-tool-subagent-report'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot");
        let rt = ctx
            .inject::<SubagentRuntime>("subagents")
            .await
            .expect("subagents");
        let parent = agents
            .create(CreateAgentOptions {
                session_id: SessionId::new(format!(
                    "parent-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_nanos())
                        .unwrap_or(0)
                )),
                cwd: Some("/work".into()),
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
            })
            .unwrap();
        parent
            .followup(Message {
                id: MessageId::new("idle"),
                role: MessageRole::User,
                content: vec![ContentBlock::Text {
                    text: "ready".into(),
                }],
                source: MessageSource::User,
            })
            .await
            .unwrap();
        parent.run_until_idle().await.unwrap();
        rt.start(
            "fork",
            SubagentStartRequest {
                label: Some("fork child".into()),
                prompt: vec![ContentBlock::Text {
                    text: "continue".into(),
                }],
                parent_id: parent.id().clone(),
                signal: AbortFlag::new(),
                max_depth: None,
            },
            parent,
        )
        .await
        .unwrap();
        let child_id = wait_child_id(&last_child).await;
        let child = agents.get(child_id.as_str()).expect("child");
        let names = child.lock().tools.lock().expect("tools").registered_names();
        assert!(
            !agents
                .tools()
                .registered_names()
                .iter()
                .any(|name| name == "report"),
            "report must not register on the shared parent runtime"
        );
        names
    }

    #[tokio::test]
    async fn report_tool_is_absent_on_one_shot_fork_child() {
        let names = tool_names_of_fork_child().await;
        assert!(!names.iter().any(|n| n == "report"));
    }

    #[test]
    fn unknown_delegate_config_key_fails_loud() {
        let err = super::resolve_config(&serde_json::json!({"provider": "spawn", "extra": true}))
            .unwrap_err();
        assert!(err.contains("unknown key"));
    }

    #[test]
    fn register_delegate_requires_known_provider() {
        let ctx = Context::new();
        let rt = SubagentRuntime::new(ctx);
        let agents = AgentRegistry::new(
            LlmRuntime::new(),
            ToolRuntime::new(ToolPresentationMode::Native),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
        );
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let err = register_delegate_tool(
            &mut tools,
            Arc::new(rt),
            Arc::new(agents),
            super::resolve_config(&serde_json::json!({"provider": "missing"})).unwrap(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown provider"));
    }
}
