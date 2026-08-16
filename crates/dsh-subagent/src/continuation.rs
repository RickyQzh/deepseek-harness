//! Continuable child publication, first-turn drive, and settlement notice.

use std::sync::{Arc, Mutex};

use dsh_agent::{AgentHandle, AgentRegistry, CreateAgentOptions};
use dsh_agent_loop::AgentStatus;
use dsh_kernel::Payload;
use dsh_session::{
    ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, SESSION_FORMAT_VERSION,
    Session, SessionEvent, SessionHeader, SessionId, SessionOrigin, TurnEndReason,
};
use dsh_tools::{AbortFlag, ToolRuntime};

use crate::{
    EVENT_SUBAGENT_END, EVENT_SUBAGENT_START, SubagentError, SubagentRunEndInfo, SubagentRunId,
    SubagentRunInfo, SubagentRuntime, SubagentStopReason, assert_subagent_max_depth,
    delegation_depth_of, snapshot_continuable_descriptor,
};

/// Contribution applied to a cloned child tool runtime before publication.
pub type ContinuableSetup = Arc<dyn Fn(&mut ToolRuntime) + Send + Sync>;

/// Inputs for one continuable in-process child start.
#[derive(Clone)]
pub struct ContinuableStartSpec {
    /// `subagents` provider name (for example `spawn`).
    pub provider: String,
    /// Durable creation label persisted on the child descriptor.
    pub label: String,
    /// Initial user prompt delivered to the child.
    pub prompt: Vec<ContentBlock>,
    /// Live parent that receives the settlement notice.
    pub parent: AgentHandle,
    /// Cancellation flag from the spawning context.
    pub signal: AbortFlag,
}

/// One live continuable child tracked for `list_agents` and `send_message`.
#[derive(Clone, Debug)]
pub struct ContinuableChildInfo {
    /// Delegating parent session id.
    pub parent: SessionId,
    /// Child session id.
    pub child: SessionId,
    /// Provider name recorded at publication.
    pub provider: String,
    /// Durable creation label.
    pub label: String,
}

pub(crate) type ContinuableRecord = ContinuableChildInfo;

struct LiveChild {
    child: AgentHandle,
    parent: AgentHandle,
    provider: String,
    seed_len: usize,
    ctx: dsh_kernel::Context,
    run_id: SubagentRunId,
}

struct ParentSnapshot {
    cwd: Option<String>,
    depth: u64,
    provider: String,
    model: String,
    max_tokens: Option<u64>,
}

impl SubagentRuntime {
    /// Register a child-local tool contribution. Never runs against the parent's shared runtime.
    pub fn register_continuable_setup(&self, setup: ContinuableSetup) {
        self.lock_setups().push(setup);
    }

    /// Continuable children of `parent`. One-shot fork runs are not listed.
    #[must_use]
    pub fn list_continuable(&self, parent: &SessionId) -> Vec<ContinuableChildInfo> {
        self.lock_continuable()
            .values()
            .filter(|row| row.parent.as_str() == parent.as_str())
            .cloned()
            .collect()
    }

    /// Publish a continuable child, drive it to idle, emit `subagent/end`, and queue the
    /// settlement notice on `parent`.
    ///
    /// Joins until that notice is queued or rejected. `subagent/end` still emits when
    /// the notice cannot be queued. Does not take the parent driver permit.
    ///
    /// # Errors
    ///
    /// Unknown provider, depth cap, missing `agents`, session/resume failure, child
    /// loop failure, or settlement notice rejection.
    pub async fn start_continuable(
        &self,
        spec: ContinuableStartSpec,
    ) -> Result<SessionId, SubagentError> {
        let live = self.publish_continuable(spec).await?;
        let id = live.child.id().clone();
        drive_and_settle(live).await?;
        Ok(id)
    }

    /// Same publication as [`start_continuable`], then drives settlement in a spawned task.
    ///
    /// Returns the child id as soon as the child is published.
    ///
    /// # Errors
    ///
    /// Same as [`start_continuable`] for publication failures.
    pub async fn start_continuable_background(
        &self,
        spec: ContinuableStartSpec,
    ) -> Result<SessionId, SubagentError> {
        let live = self.publish_continuable(spec).await?;
        let id = live.child.id().clone();
        tokio::spawn(async move {
            let _ = drive_and_settle(live).await;
        });
        Ok(id)
    }

    /// Queue `message` on a continuable child. When the child is idle, drive only
    /// that child with [`AgentHandle::run_until_idle`]. A running child picks the
    /// mail up after its current turn. Never takes the parent driver permit.
    ///
    /// # Errors
    ///
    /// Unknown child, parent mismatch, missing `agents`, inbox splice failure, or
    /// child loop failure.
    pub async fn followup_child(
        &self,
        child_id: &SessionId,
        message: Message,
        parent: &AgentHandle,
    ) -> Result<(), SubagentError> {
        let record = {
            let table = self.lock_continuable();
            table.get(child_id.as_str()).cloned()
        };
        let Some(record) = record else {
            return Err(SubagentError::other(format!(
                "unknown continuable child `{}`",
                child_id.as_str()
            )));
        };
        if record.parent.as_str() != parent.id().as_str() {
            return Err(SubagentError::other(format!(
                "child `{}` is not a child of this parent",
                child_id.as_str()
            )));
        }
        let agents = self
            .context()
            .inject::<AgentRegistry>("agents")
            .await
            .map_err(|error| SubagentError::other(error.to_string()))?;
        let child = agents.get(child_id.as_str()).ok_or_else(|| {
            SubagentError::other(format!("unknown continuable child `{}`", child_id.as_str()))
        })?;
        child
            .followup(message)
            .await
            .map_err(|error| SubagentError::other(error.to_string()))?;
        let idle = {
            let agent = child.lock();
            agent.status() == AgentStatus::Idle
        };
        if idle {
            child
                .run_until_idle()
                .await
                .map_err(|error| SubagentError::other(error.to_string()))?;
        }
        Ok(())
    }

    async fn publish_continuable(
        &self,
        spec: ContinuableStartSpec,
    ) -> Result<LiveChild, SubagentError> {
        if spec.signal.is_aborted() {
            return Err(SubagentError::other(
                "subagent request was aborted before child publication",
            ));
        }
        let selected = self
            .get_provider(&spec.provider)
            .ok_or_else(|| SubagentError::UnknownProvider(spec.provider.clone()))?;
        let parent_id = spec.parent.id().clone();
        let snapshot = {
            let agent = spec.parent.lock();
            ParentSnapshot {
                cwd: agent.session.header().cwd.clone(),
                depth: delegation_depth_of(agent.session.header()),
                provider: agent.options.provider.clone(),
                model: agent.options.model.clone(),
                max_tokens: agent.options.max_tokens,
            }
        };
        let create = selected.prepare_continuable(&spec.parent);
        let child_depth = assert_subagent_max_depth(snapshot.depth, None)?;
        let agents = self
            .context()
            .inject::<AgentRegistry>("agents")
            .await
            .map_err(|error| SubagentError::other(error.to_string()))?;
        let child_id = mint_child_id();
        let seed = create.seed.unwrap_or_default();
        let seed_len = seed.len();
        let mut session = if seed.is_empty() {
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
                    parent_id.clone(),
                    snapshot.cwd.clone(),
                    child_depth,
                    seed_len,
                ),
                seed,
            )
            .map_err(|error| SubagentError::other(error.to_string()))?
        };
        append_continuable_descriptor(&mut session, &spec.provider, &spec.label)?;
        let mut child_tools = agents.tools().clone();
        {
            let setups = self.lock_setups();
            for setup in setups.iter() {
                setup(&mut child_tools);
            }
        }
        let child = agents
            .resume_with_tools(
                session,
                CreateAgentOptions {
                    session_id: child_id.clone(),
                    cwd: snapshot.cwd,
                    provider: snapshot.provider,
                    model: snapshot.model,
                    max_tokens: snapshot.max_tokens,
                },
                Some(Arc::new(Mutex::new(child_tools))),
            )
            .map_err(|error| SubagentError::other(error.to_string()))?;
        {
            let mut table = self.lock_continuable();
            table.insert(
                child_id.as_str().to_string(),
                ContinuableChildInfo {
                    parent: parent_id.clone(),
                    child: child_id.clone(),
                    provider: spec.provider.clone(),
                    label: spec.label.clone(),
                },
            );
        }
        let run_id = SubagentRunId::new(child_id.as_str());
        self.context().emit(
            EVENT_SUBAGENT_START,
            Payload::new(SubagentRunInfo {
                run_id: run_id.clone(),
                provider: spec.provider.clone(),
                id: child_id.clone(),
                parent: parent_id,
            }),
        );
        child
            .followup(user_prompt(&child_id, spec.prompt))
            .await
            .map_err(|error| SubagentError::other(error.to_string()))?;
        Ok(LiveChild {
            child,
            parent: spec.parent,
            provider: spec.provider,
            seed_len,
            ctx: self.context().clone(),
            run_id,
        })
    }
}

async fn drive_and_settle(live: LiveChild) -> Result<(), SubagentError> {
    let driven = live.child.run_until_idle().await;
    let (output, mut stop_reason) = {
        let agent = live.child.lock();
        let events = agent.session.events();
        let start = live.seed_len.min(events.len());
        let suffix = &events[start..];
        (suffix_output(suffix), suffix_stop_reason(suffix))
    };
    if driven.is_err() {
        stop_reason = SubagentStopReason::Error;
    }
    let notified = notify_settlement(&live.parent, live.child.id(), stop_reason, &output).await;
    live.ctx.emit(
        EVENT_SUBAGENT_END,
        Payload::new(SubagentRunEndInfo {
            run_id: live.run_id,
            provider: live.provider,
            id: live.child.id().clone(),
            parent: live.parent.id().clone(),
            stop_reason,
        }),
    );
    notified?;
    driven.map_err(|error| SubagentError::other(error.to_string()))
}

async fn notify_settlement(
    parent: &AgentHandle,
    child_id: &SessionId,
    stop_reason: SubagentStopReason,
    output: &[ContentBlock],
) -> Result<(), SubagentError> {
    let summary = settlement_summary(child_id, stop_reason);
    let mut content = vec![ContentBlock::Text {
        text: summary.clone(),
    }];
    if output.is_empty() {
        content.push(ContentBlock::Text {
            text: "It left no closing message.".into(),
        });
    } else {
        content.push(ContentBlock::Text {
            text: "Its closing message:".into(),
        });
        content.extend(output.iter().cloned());
    }
    parent
        .followup(Message {
            id: MessageId::new(format!("{}-settled", child_id.as_str())),
            role: MessageRole::User,
            content,
            source: MessageSource::SubagentSettled {
                form: "notice".into(),
                summary,
                sender_session_id: child_id.as_str().to_string(),
            },
        })
        .await
        .map_err(|error| SubagentError::other(error.to_string()))?;
    Ok(())
}

fn settlement_summary(child_id: &SessionId, stop_reason: SubagentStopReason) -> String {
    let subject = format!("Background subagent {}", child_id.as_str());
    match stop_reason {
        SubagentStopReason::Completed => {
            format!("{subject} finished and will do no further work unless you send it more.")
        }
        SubagentStopReason::Aborted => format!("{subject} was stopped before it finished."),
        SubagentStopReason::MaxTokens => {
            format!("{subject} ran out of room before it finished.")
        }
        SubagentStopReason::Refusal => format!("{subject} declined the task."),
        SubagentStopReason::Error => format!("{subject} failed before it finished."),
    }
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

fn append_continuable_descriptor(
    session: &mut Session,
    provider: &str,
    label: &str,
) -> Result<(), SubagentError> {
    let seq = session.events().len() as u64;
    session
        .append(SessionEvent::SubagentDescriptor {
            seq,
            time: seq as i64,
            data: snapshot_continuable_descriptor(provider, label),
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
    use super::ContinuableStartSpec;
    use crate::{
        ContinuableCreateSpec, EVENT_SUBAGENT_END, EVENT_SUBAGENT_START, SubagentCapabilities,
        SubagentError, SubagentProvider, SubagentResult, SubagentRunEndInfo, SubagentRunInfo,
        SubagentRuntime, SubagentStartRequest, SubagentStopReason,
    };
    use dsh_agent::{AgentHandle, AgentRegistry, CreateAgentOptions};
    use dsh_agent_loop::{AgentStatus, CancelCause, CancelOptions};
    use dsh_kernel::Context;
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response};
    use dsh_session::{ContentBlock, Message, MessageId, MessageRole, MessageSource, SessionId};
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{AbortFlag, ToolPresentationMode, ToolRuntime};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct FakeSpawn;

    impl SubagentProvider for FakeSpawn {
        fn name(&self) -> &str {
            "spawn"
        }

        fn capabilities(&self) -> SubagentCapabilities {
            SubagentCapabilities::all()
        }

        fn inherits_parent_context(&self) -> bool {
            false
        }

        fn start(
            &self,
            _request: SubagentStartRequest,
            _parent: AgentHandle,
        ) -> Pin<Box<dyn Future<Output = Result<SubagentResult, SubagentError>> + Send + '_>>
        {
            Box::pin(async { Err(SubagentError::other("one-shot start is unused")) })
        }

        fn prepare_continuable(&self, _parent: &AgentHandle) -> ContinuableCreateSpec {
            ContinuableCreateSpec { seed: None }
        }
    }

    struct Env {
        rt: SubagentRuntime,
        agents: Arc<AgentRegistry>,
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    }

    fn text_blocks(text: &str) -> Vec<ContentBlock> {
        vec![ContentBlock::Text { text: text.into() }]
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

    fn source_kind(source: &MessageSource) -> Option<String> {
        serde_json::to_value(source)
            .ok()
            .and_then(|value| value.get("kind")?.as_str().map(str::to_string))
    }

    fn parent_has_source_kind(parent: &AgentHandle, kind: &str) -> bool {
        parent
            .lock()
            .session
            .derive_messages()
            .iter()
            .any(|message| source_kind(&message.source).as_deref() == Some(kind))
    }

    fn parent_text_contains(parent: &AgentHandle, needle: &str) -> bool {
        handle_text_contains(parent, needle)
    }

    fn handle_text_contains(handle: &AgentHandle, needle: &str) -> bool {
        handle
            .lock()
            .session
            .derive_messages()
            .iter()
            .any(|message| message_text(message).contains(needle))
    }

    async fn boot_continuable() -> Env {
        boot_continuable_with(vec![
            MockScript::Chunks(text_response("parent-idle")),
            MockScript::Chunks(text_response("CHILD_RESULT")),
            MockScript::Chunks(text_response("parent-settled")),
        ])
        .await
    }

    async fn boot_continuable_with(script: Vec<MockScript>) -> Env {
        let adapter = Arc::new(MockAdapter::new(script));
        let ctx = Context::new();
        let llm = Arc::new(Mutex::new(LlmRuntime::new()));
        llm.lock().expect("llm").register_adapter("mock", adapter);
        let tools = Arc::new(Mutex::new(ToolRuntime::new(ToolPresentationMode::Native)));
        let prompt = SystemPrompt::new(SystemPromptConfig::default()).unwrap();
        ctx.provide(
            "agents",
            AgentRegistry::from_shared(ctx.clone(), llm, tools, prompt),
        )
        .expect("provide agents");
        let agents = ctx.inject::<AgentRegistry>("agents").await.expect("agents");
        let rt = SubagentRuntime::new(ctx);
        rt.register_provider(Arc::new(FakeSpawn)).unwrap();
        Env { rt, agents }
    }

    async fn idle_parent(env: &Env) -> AgentHandle {
        let parent = env
            .agents
            .create(CreateAgentOptions {
                session_id: SessionId::new(format!("parent-{}", unique_suffix())),
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
                content: text_blocks("ready"),
                source: MessageSource::User,
            })
            .await
            .unwrap();
        parent.run_until_idle().await.unwrap();
        parent
    }

    #[tokio::test]
    async fn continuable_settlement_injects_subagent_settled_notice() {
        let env = boot_continuable().await;
        let parent = idle_parent(&env).await;
        let child_id = env
            .rt
            .start_continuable(ContinuableStartSpec {
                provider: "spawn".into(),
                label: "Return child result".into(),
                prompt: text_blocks("Reply with exactly CHILD_RESULT"),
                parent: parent.clone(),
                signal: AbortFlag::new(),
            })
            .await
            .unwrap();
        parent.run_until_idle().await.unwrap();
        assert!(parent_has_source_kind(&parent, "subagent-settled"));
        assert!(parent_text_contains(&parent, "CHILD_RESULT"));
        let _ = child_id;
    }

    #[tokio::test]
    async fn followup_child_drives_idle_child_for_later_turn() {
        let env = boot_continuable_with(vec![
            MockScript::Chunks(text_response("parent-idle")),
            MockScript::Chunks(text_response("CHILD_RESULT")),
            MockScript::Chunks(text_response("SECOND_TURN")),
        ])
        .await;
        let parent = idle_parent(&env).await;
        let child_id = env
            .rt
            .start_continuable(ContinuableStartSpec {
                provider: "spawn".into(),
                label: "Return child result".into(),
                prompt: text_blocks("Reply with exactly CHILD_RESULT"),
                parent: parent.clone(),
                signal: AbortFlag::new(),
            })
            .await
            .unwrap();
        env.rt
            .followup_child(
                &child_id,
                Message {
                    id: MessageId::new("coord-1"),
                    role: MessageRole::User,
                    content: text_blocks("do a second turn"),
                    source: MessageSource::Coordinator {
                        form: "relay".into(),
                        sender_session_id: parent.id().as_str().to_string(),
                    },
                },
                &parent,
            )
            .await
            .unwrap();
        let child = env.agents.get(child_id.as_str()).expect("child");
        assert!(
            handle_text_contains(&child, "SECOND_TURN"),
            "idle continuable child must run the followup turn"
        );
    }

    #[tokio::test]
    async fn emits_subagent_end_when_settlement_followup_fails() {
        let env = boot_continuable_with(vec![
            MockScript::Chunks(text_response("parent-idle")),
            MockScript::Hang,
        ])
        .await;
        let starts = Arc::new(Mutex::new(Vec::<SessionId>::new()));
        let ends = Arc::new(Mutex::new(Vec::<SubagentStopReason>::new()));
        let start_slot = Arc::clone(&starts);
        let end_slot = Arc::clone(&ends);
        env.rt
            .context()
            .on(EVENT_SUBAGENT_START, move |payload| {
                let start_slot = Arc::clone(&start_slot);
                let id = payload
                    .downcast_ref::<SubagentRunInfo>()
                    .map(|info| info.id.clone());
                async move {
                    if let Some(id) = id {
                        start_slot
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(id);
                    }
                }
            })
            .expect("listen start");
        env.rt
            .context()
            .on(EVENT_SUBAGENT_END, move |payload| {
                let end_slot = Arc::clone(&end_slot);
                let reason = payload
                    .downcast_ref::<SubagentRunEndInfo>()
                    .map(|info| info.stop_reason);
                async move {
                    if let Some(reason) = reason {
                        end_slot
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(reason);
                    }
                }
            })
            .expect("listen end");
        let parent = idle_parent(&env).await;
        let start_fut = env.rt.start_continuable(ContinuableStartSpec {
            provider: "spawn".into(),
            label: "hang then fail notify".into(),
            prompt: text_blocks("hang"),
            parent: parent.clone(),
            signal: AbortFlag::new(),
        });
        tokio::pin!(start_fut);
        let child_id = wait_while_driving(&mut start_fut, || {
            starts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .first()
                .cloned()
        })
        .await;
        wait_while_driving(&mut start_fut, || {
            env.agents
                .get(child_id.as_str())
                .map(|child| child.lock().status() == AgentStatus::Running)
                .unwrap_or(false)
                .then_some(())
        })
        .await;
        parent
            .followup(Message {
                id: MessageId::new(format!("{}-settled", child_id.as_str())),
                role: MessageRole::User,
                content: text_blocks("collide"),
                source: MessageSource::User,
            })
            .await
            .unwrap();
        {
            let child = env.agents.get(child_id.as_str()).expect("child");
            child
                .lock()
                .cancel(CancelCause::User, CancelOptions::default())
                .unwrap();
        }
        let outcome = tokio::time::timeout(Duration::from_secs(2), start_fut)
            .await
            .expect("start_continuable should finish after child abort");
        assert!(outcome.is_err(), "{outcome:?}");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let end_reasons = ends
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if !end_reasons.is_empty() {
                assert_eq!(end_reasons.len(), 1, "subagent/end must pair start");
                assert_eq!(
                    starts
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .len(),
                    1
                );
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("timed out waiting for subagent/end after settlement followup failure");
            }
            tokio::task::yield_now().await;
        }
    }

    async fn wait_while_driving<T>(
        start_fut: &mut std::pin::Pin<
            &mut impl std::future::Future<Output = Result<SessionId, SubagentError>>,
        >,
        mut ready: impl FnMut() -> Option<T>,
    ) -> T {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(value) = ready() {
                return value;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("timed out waiting while driving start_continuable");
            }
            tokio::select! {
                biased;
                result = start_fut.as_mut() => {
                    panic!("start_continuable returned before the test fixture was ready: {result:?}");
                }
                () = tokio::time::sleep(Duration::from_millis(1)) => {}
            }
        }
    }
}
