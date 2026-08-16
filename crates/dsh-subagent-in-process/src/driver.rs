//! Shared one-shot in-process child driver.

use dsh_agent::{AgentHandle, AgentRegistry, CreateAgentOptions};
use dsh_kernel::{Context, Payload};
use dsh_session::{
    ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, SESSION_FORMAT_VERSION,
    Session, SessionEvent, SessionHeader, SessionId, SessionOrigin, TurnEndReason,
};
use dsh_subagent::{
    EVENT_SUBAGENT_END, EVENT_SUBAGENT_START, SubagentError, SubagentResult, SubagentRunEndInfo,
    SubagentRunId, SubagentRunInfo, SubagentStartRequest, SubagentStopReason,
    assert_subagent_max_depth, delegation_depth_of, snapshot_one_shot_descriptor,
};

/// Events through the last `turn/end` inclusive. Empty when no turn has completed.
#[must_use]
pub(crate) fn completed_turn_prefix(session: &Session) -> Vec<LogEvent> {
    let events = session.events();
    match events
        .iter()
        .rposition(|event| event.event_type() == "turn/end")
    {
        Some(index) => events[..=index].to_vec(),
        None => Vec::new(),
    }
}

/// Establish and drive one in-process one-shot child. Never takes the parent driver permit.
///
/// # Errors
///
/// Depth cap, missing `agents`, session/resume failure, or child loop failure.
pub async fn start_in_process_run(
    ctx: &Context,
    request: SubagentStartRequest,
    parent: AgentHandle,
    provider_name: &str,
    inherit_parent_context: bool,
) -> Result<SubagentResult, SubagentError> {
    if request.signal.is_aborted() {
        return Err(SubagentError::other(
            "subagent request was aborted before child publication",
        ));
    }
    let parent_id = parent.id().clone();
    let snapshot = {
        let agent = parent.lock();
        let seed = if inherit_parent_context {
            completed_turn_prefix(&agent.session)
        } else {
            Vec::new()
        };
        ParentSnapshot {
            cwd: agent.session.header().cwd.clone(),
            depth: delegation_depth_of(agent.session.header()),
            provider: agent.options.provider.clone(),
            model: agent.options.model.clone(),
            max_tokens: agent.options.max_tokens,
            seed,
        }
    };
    let child_depth = assert_subagent_max_depth(snapshot.depth, request.max_depth)?;
    let agents = ctx
        .inject::<AgentRegistry>("agents")
        .await
        .map_err(|error| SubagentError::other(error.to_string()))?;
    let child_id = mint_child_id();
    let seed_len = snapshot.seed.len();
    let mut session = if snapshot.seed.is_empty() {
        Session::new(child_header(
            child_id.clone(),
            parent_id.clone(),
            snapshot.cwd.clone(),
            child_depth,
            0,
        ))
    } else {
        Session::from_events(
            child_header(
                child_id.clone(),
                parent_id,
                snapshot.cwd.clone(),
                child_depth,
                seed_len,
            ),
            snapshot.seed,
        )
        .map_err(|error| SubagentError::other(error.to_string()))?
    };
    append_one_shot_descriptor(&mut session, provider_name, request.label.as_deref())?;
    let child = agents
        .resume(
            session,
            CreateAgentOptions {
                session_id: child_id.clone(),
                cwd: snapshot.cwd,
                provider: snapshot.provider,
                model: snapshot.model,
                max_tokens: snapshot.max_tokens,
            },
        )
        .map_err(|error| SubagentError::other(error.to_string()))?;
    let run_id = SubagentRunId::new(child_id.as_str());
    let info = SubagentRunInfo {
        run_id: run_id.clone(),
        provider: provider_name.to_string(),
        id: child_id.clone(),
    };
    ctx.emit(EVENT_SUBAGENT_START, Payload::new(info));
    child
        .followup(user_prompt(&child_id, request.prompt))
        .await
        .map_err(|error| SubagentError::other(error.to_string()))?;
    child
        .run_until_idle()
        .await
        .map_err(|error| SubagentError::other(error.to_string()))?;
    let (output, stop_reason) = {
        let agent = child.lock();
        let events = agent.session.events();
        let start = seed_len.min(events.len());
        let suffix = &events[start..];
        (suffix_output(suffix), suffix_stop_reason(suffix))
    };
    ctx.emit(
        EVENT_SUBAGENT_END,
        Payload::new(SubagentRunEndInfo {
            run_id,
            provider: provider_name.to_string(),
            id: child_id,
            stop_reason,
        }),
    );
    Ok(SubagentResult {
        stop_reason,
        output,
    })
}

struct ParentSnapshot {
    cwd: Option<String>,
    depth: u64,
    provider: String,
    model: String,
    max_tokens: Option<u64>,
    seed: Vec<LogEvent>,
}

fn mint_child_id() -> SessionId {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    SessionId::new(format!("sub-{pid}-{nanos}"))
}

fn child_header(
    child_id: SessionId,
    parent_id: SessionId,
    cwd: Option<String>,
    depth: u64,
    seed_len: usize,
) -> SessionHeader {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let seed_length = if seed_len == 0 {
        None
    } else {
        Some(seed_len as u64)
    };
    SessionHeader {
        version: SESSION_FORMAT_VERSION,
        id: child_id,
        created_at,
        cwd,
        parent_session: Some(parent_id),
        seed_length,
        origin: Some(SessionOrigin::Subagent),
        delegation_depth: Some(depth),
        agent_preset: None,
    }
}

fn append_one_shot_descriptor(
    session: &mut Session,
    provider: &str,
    label: Option<&str>,
) -> Result<(), SubagentError> {
    let seq = session.events().len() as u64;
    session
        .append(SessionEvent::SubagentDescriptor {
            seq,
            time: seq as i64,
            data: snapshot_one_shot_descriptor(provider, label),
            ignorable: None,
        })
        .map_err(|error| SubagentError::other(error.to_string()))?;
    Ok(())
}

fn user_prompt(child_id: &SessionId, prompt: Vec<ContentBlock>) -> Message {
    Message {
        id: MessageId::new(format!("{}-prompt", child_id.as_str())),
        role: MessageRole::User,
        content: prompt,
        source: MessageSource::User,
    }
}

fn suffix_output(events: &[LogEvent]) -> Vec<ContentBlock> {
    let mut output = Vec::new();
    for event in events {
        if let LogEvent::Known(SessionEvent::AssistantMessage { data, .. }) = event {
            if !data.message.content.is_empty() {
                output = data.message.content.clone();
            }
        }
    }
    output
}

fn suffix_stop_reason(events: &[LogEvent]) -> SubagentStopReason {
    let mut last = None;
    for event in events {
        if let LogEvent::Known(SessionEvent::TurnEnd { data, .. }) = event {
            last = Some(&data.reason);
        }
    }
    match last {
        Some(TurnEndReason::Completed) => SubagentStopReason::Completed,
        Some(TurnEndReason::Aborted { .. }) => SubagentStopReason::Aborted,
        Some(TurnEndReason::Error { .. }) => SubagentStopReason::Error,
        Some(TurnEndReason::MaxTokens) => SubagentStopReason::MaxTokens,
        Some(TurnEndReason::Blocked) => SubagentStopReason::Refusal,
        Some(TurnEndReason::Interrupted) | None => SubagentStopReason::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::completed_turn_prefix;
    use crate::{register_fork, register_spawn};
    use dsh_agent::{AgentHandle, AgentRegistry, CreateAgentOptions};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response, tool_call_response};
    use dsh_session::{
        ContentBlock, Message, MessageId, MessageRole, MessageSource, SESSION_FORMAT_VERSION,
        Session, SessionHeader, SessionId, SessionOrigin,
    };
    use dsh_subagent::{
        EVENT_SUBAGENT_START, SubagentRunInfo, SubagentRuntime, SubagentStartRequest,
        SubagentStopReason,
    };
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{AbortFlag, ToolDefinition, ToolError, ToolPresentationMode, ToolRuntime};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct Env {
        rt: Arc<SubagentRuntime>,
        agents: Arc<AgentRegistry>,
        last_child: Arc<Mutex<Option<SessionId>>>,
    }

    impl Env {
        async fn parent_with_history(&self, text: &str) -> AgentHandle {
            let parent = self
                .agents
                .create(CreateAgentOptions {
                    session_id: SessionId::new(format!("parent-{}", unique_suffix())),
                    cwd: Some("/work".into()),
                    provider: "mock".into(),
                    model: "mock".into(),
                    max_tokens: None,
                })
                .unwrap();
            parent.followup(user_text("m1", text)).await.unwrap();
            parent.run_until_idle().await.unwrap();
            parent
        }

        async fn child_session(&self, parent: &AgentHandle) -> Session {
            let id = wait_child_id(&self.last_child).await;
            let handle = self.agents.get(id.as_str()).expect("child agent");
            let session = handle.lock().session.clone();
            assert_eq!(
                session
                    .header()
                    .parent_session
                    .as_ref()
                    .map(SessionId::as_str),
                Some(parent.id().as_str())
            );
            session
        }
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    }

    fn user_text(id: &str, text: &str) -> Message {
        Message {
            id: MessageId::new(id),
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            source: MessageSource::User,
        }
    }

    fn prompt(text: &str) -> SubagentStartRequest {
        SubagentStartRequest {
            label: None,
            prompt: vec![ContentBlock::Text { text: text.into() }],
            parent_id: SessionId::new("pending"),
            signal: AbortFlag::new(),
            max_depth: None,
        }
    }

    fn message_text(message: &Message) -> String {
        message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    async fn wait_child_id(slot: &Mutex<Option<SessionId>>) -> SessionId {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(id) = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                return id;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("timed out waiting for subagent/start");
            }
            tokio::task::yield_now().await;
        }
    }

    async fn boot_subagents() -> Env {
        let adapter = Arc::new(MockAdapter::new(vec![
            MockScript::Chunks(text_response("parent-ack")),
            MockScript::Chunks(text_response("child-ok")),
            MockScript::Chunks(text_response("extra")),
        ]));
        boot_with(adapter, |_| {}).await
    }

    async fn boot_with(
        adapter: Arc<MockAdapter>,
        configure_tools: impl FnOnce(&mut ToolRuntime),
    ) -> Env {
        let ctx = Context::new();
        let llm = Arc::new(Mutex::new(LlmRuntime::new()));
        llm.lock().expect("llm").register_adapter("mock", adapter);
        let tools = Arc::new(Mutex::new(ToolRuntime::new(ToolPresentationMode::Native)));
        configure_tools(&mut tools.lock().expect("tools"));
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
            let info = payload.downcast_ref::<SubagentRunInfo>().cloned();
            let slot = Arc::clone(&slot);
            async move {
                if let Some(info) = info {
                    *slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(info.id);
                }
            }
        })
        .expect("listen");
        let mut registry = PluginRegistry::new();
        dsh_subagent::register(&mut registry);
        register_spawn(&mut registry);
        register_fork(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-subagent'\n- name: '@deepseek-ai/dsh-subagent-spawn-in-process'\n- name: '@deepseek-ai/dsh-subagent-fork-in-process'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot subagents");
        let rt = ctx
            .inject::<SubagentRuntime>("subagents")
            .await
            .expect("subagents");
        Env {
            rt,
            agents,
            last_child,
        }
    }

    async fn running_parent_with_hanging_tool() -> (Env, AgentHandle) {
        let adapter = Arc::new(MockAdapter::new(vec![
            MockScript::Chunks(tool_call_response("c1", "hang", &json!({}), None)),
            MockScript::Chunks(text_response("hi")),
        ]));
        let env = boot_with(adapter, |tools| {
            tools.register(ToolDefinition {
                name: "hang".into(),
                description: "never returns".into(),
                parameters: json!({"type": "object"}),
                execute: Box::new(|_args, exec| {
                    Box::pin(async move {
                        exec.signal.cancelled().await;
                        Err(ToolError::Other("hung until cancel".into()))
                    })
                }),
                render: Box::new(|_, _| Vec::new()),
                is_concurrency_safe: None,
            });
        })
        .await;
        let parent = env
            .agents
            .create(CreateAgentOptions {
                session_id: SessionId::new("hang-parent"),
                cwd: None,
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
            })
            .unwrap();
        parent.followup(user_text("go", "go")).await.unwrap();
        (env, parent)
    }

    #[tokio::test]
    async fn spawn_child_does_not_see_parent_history() {
        let env = boot_subagents().await;
        let parent = env.parent_with_history("secret parent fact").await;
        let result = env
            .rt
            .start("spawn", prompt("repeat any secret"), parent.clone())
            .await
            .unwrap();
        assert!(matches!(result.stop_reason, SubagentStopReason::Completed));
        let child = env.child_session(&parent).await;
        assert!(
            !child
                .derive_messages()
                .iter()
                .any(|message| message_text(message).contains("secret parent fact"))
        );
        assert_eq!(
            child.header().origin.as_ref(),
            Some(&SessionOrigin::Subagent)
        );
        assert_eq!(child.header().delegation_depth, Some(1));
        assert_eq!(child.header().seed_length, None);
        assert!(
            child
                .events()
                .iter()
                .any(|event| event.event_type() == "subagent/descriptor")
        );
    }

    #[tokio::test]
    async fn fork_child_is_seeded_with_completed_parent_turns() {
        let env = boot_subagents().await;
        let parent = env.parent_with_history("inherited").await;
        let parent_prefix = completed_turn_prefix(&parent.lock().session);
        assert!(!parent_prefix.is_empty());
        let _ = env
            .rt
            .start("fork", prompt("ok"), parent.clone())
            .await
            .unwrap();
        let child = env.child_session(&parent).await;
        assert!(
            child
                .derive_messages()
                .iter()
                .any(|message| message_text(message).contains("inherited"))
        );
        assert_eq!(child.header().seed_length, Some(parent_prefix.len() as u64));
    }

    #[tokio::test]
    async fn start_does_not_hold_parent_driver_permit() {
        let (env, parent) = running_parent_with_hanging_tool().await;
        let driver = tokio::spawn({
            let handle = parent.clone();
            async move { handle.run_until_idle().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let child = tokio::time::timeout(
            Duration::from_secs(2),
            env.rt.start("spawn", prompt("hi"), parent.clone()),
        )
        .await;
        assert!(
            child.is_ok(),
            "child start timed out; parent driver was held"
        );
        assert!(child.unwrap().is_ok());
        parent.cancel().await.unwrap();
        let _ = driver.await;
    }

    #[tokio::test]
    async fn depth_cap_fails_when_exceeded() {
        let env = boot_subagents().await;
        let session = Session::new(SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("deep-parent"),
            created_at: 1,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: Some(3),
            agent_preset: None,
        });
        let parent = env
            .agents
            .resume(
                session,
                CreateAgentOptions {
                    session_id: SessionId::new("deep-parent"),
                    cwd: None,
                    provider: "mock".into(),
                    model: "mock".into(),
                    max_tokens: None,
                },
            )
            .unwrap();
        let err = env
            .rt
            .start("spawn", prompt("hi"), parent)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("exceeds maxDepth 3"), "{err}");
    }
}
