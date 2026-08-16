//! Kernel plugin `@deepseek-ai/dsh-agent-instructions`.

use std::path::PathBuf;
use std::sync::Arc;

use dsh_agent_loop::{CompactionScope, EVENT_AGENT_PRE_STEP, PreStepDecision};
use dsh_boot::{PLUGIN_AGENT_INSTRUCTIONS, PluginRegistry, PluginSetup};
use dsh_fs::LocalFileSystem;
use dsh_kernel::KernelError;
use dsh_session::{
    ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, Session, SessionEvent,
};
use serde_json::Value;

use crate::config::{AgentInstructionsConfig, resolve_config};
use crate::files::{find_project_root, load_baseline_files};
use crate::render::{
    instruction_content_sha1, instruction_scope_key, render_workspace_context,
    workspace_baseline_identity,
};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register the pre-step listener. Unknown keys and a missing `maxBytes` fail load.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved = resolve_config(&config).map_err(|error| setup_err(error.to_string()))?;
            let pre_ctx = ctx.clone();
            ctx.on_waterfall::<PreStepDecision, _, _>(
                EVENT_AGENT_PRE_STEP,
                move |decision, next| {
                    let ctx = pre_ctx.clone();
                    let config = resolved.clone();
                    async move {
                        let decision = match decision {
                            PreStepDecision::Enter { mut messages } if !messages.is_empty() => {
                                if let Some(message) = maybe_baseline_message(&ctx, &config).await {
                                    messages.insert(0, message);
                                }
                                PreStepDecision::Enter { messages }
                            }
                            other => other,
                        };
                        next(decision).await
                    }
                },
            )
            .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_AGENT_INSTRUCTIONS, setup);
}

async fn maybe_baseline_message(
    ctx: &dsh_kernel::Context,
    config: &AgentInstructionsConfig,
) -> Option<Message> {
    let (cwd, visible_identity) = CompactionScope::try_current(|scope| {
        scope.with_session(|session| {
            (
                session.header().cwd.clone(),
                visible_baseline_identity(session),
            )
        })
    })?;
    let cwd = PathBuf::from(cwd?);
    let fs = ctx.get::<LocalFileSystem>("fs");
    let files = load_baseline_files(&cwd, config, fs.as_deref()).await;
    if files.is_empty() && visible_identity.is_none() {
        return None;
    }
    let project_root = find_project_root(&cwd, &config.project_root_markers, fs.as_deref()).await;
    let identity = workspace_baseline_identity(config, &cwd, &project_root, &files);
    let replace_previous = match &visible_identity {
        None => false,
        Some(previous) if previous.as_deref() == Some(identity.as_str()) => return None,
        Some(_) => true,
    };
    let text = render_workspace_context(&files, config.max_bytes, replace_previous);
    if text.is_empty() {
        return None;
    }
    let changes: Vec<Value> = files
        .iter()
        .map(|file| {
            serde_json::json!({
                "action": "set",
                "scope": instruction_scope_key(&file.display_path),
                "path": file.display_path,
                "digest": instruction_content_sha1(&file.content),
            })
        })
        .collect();
    Some(Message {
        id: mint_message_id(),
        role: MessageRole::User,
        content: vec![ContentBlock::Text { text }],
        source: MessageSource::AgentInstructions {
            form: "instructions".into(),
            baseline: Some(true),
            baseline_identity: Some(identity),
            changes,
        },
    })
}

fn visible_baseline_identity(session: &Session) -> Option<Option<String>> {
    for seq in session.surface_nodes().iter().rev() {
        let Some(LogEvent::Known(SessionEvent::UserMessage { data, .. })) =
            session.events().get(*seq as usize)
        else {
            continue;
        };
        if let MessageSource::AgentInstructions {
            baseline: Some(true),
            baseline_identity,
            ..
        } = &data.source
        {
            return Some(baseline_identity.clone());
        }
    }
    None
}

fn mint_message_id() -> MessageId {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    MessageId::new(format!(
        "agent-instructions-{}-{}",
        std::process::id(),
        nanos
    ))
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::resolve_config;
    use dsh_agent::{AgentHandle, AgentRegistry, CreateAgentOptions};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_session::{
        ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource,
        SESSION_FORMAT_VERSION, Session, SessionEvent, SessionHeader, SessionId, SurfaceOp,
    };
    use dsh_session_persist::JsonlSessionStore;
    use serde_json::json;
    use std::path::{Path, PathBuf};

    fn test_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn user_text(id: &str, text: &str) -> Message {
        Message {
            id: MessageId::new(id),
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            source: MessageSource::User,
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

    fn first_user_source(agent: &AgentHandle) -> MessageSource {
        let guard = agent.lock();
        for event in guard.session.events() {
            if let LogEvent::Known(SessionEvent::UserMessage { data, .. }) = event {
                return data.source.clone();
            }
        }
        panic!("no user/message in session");
    }

    fn instruction_yaml(dsh_home: &Path) -> String {
        format!(
            "\
- name: '@deepseek-ai/dsh-llm'
- name: '@deepseek-ai/dsh-llm-mock'
  config:
    provider: mock
    text: instr-ok
- name: '@deepseek-ai/dsh-tools'
- name: '@deepseek-ai/dsh-system-prompt'
  config:
    includeRuntimeContext: false
    includeHarnessIdentity: false
- name: '@deepseek-ai/dsh-agent'
- name: '@deepseek-ai/dsh-agent-instructions'
  config:
    maxBytes: 65536
    dshHome: {}
",
            json!(dsh_home.to_string_lossy().as_ref())
        )
    }

    fn register_stack(registry: &mut PluginRegistry) {
        dsh_llm::plugin::register_llm(registry);
        dsh_llm::plugin::register_mock(registry);
        dsh_tools::plugin::register(registry);
        dsh_system_prompt::plugin::register(registry);
        dsh_agent::plugin::register(registry);
        register(registry);
    }

    async fn boot_instructions_agent(root: &Path) -> AgentHandle {
        let home = root.join(".dsh-home");
        std::fs::create_dir_all(&home).expect("dsh home");
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_stack(&mut registry);
        boot_yaml(
            &ctx,
            &instruction_yaml(&home),
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .unwrap();
        let agents = ctx.get::<AgentRegistry>("agents").expect("agents");
        agents
            .create(CreateAgentOptions {
                session_id: SessionId::new("instr-session"),
                cwd: Some(root.to_string_lossy().into_owned()),
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
            })
            .unwrap()
    }

    #[tokio::test]
    async fn baseline_injects_agents_md_before_first_request() {
        let root = test_temp_dir("instr");
        std::fs::write(root.join("AGENTS.md"), "Be concise.\n").unwrap();
        std::fs::write(root.join(".git"), "").unwrap();
        let agent = boot_instructions_agent(&root).await;
        agent.followup(user_text("hi", "hi")).await.unwrap();
        agent.run_until_idle().await.unwrap();
        let src = first_user_source(&agent);
        assert!(matches!(
            src,
            MessageSource::AgentInstructions {
                baseline: Some(true),
                ..
            }
        ));
        let guard = agent.lock();
        let text = guard
            .session
            .events()
            .iter()
            .find_map(|event| match event {
                LogEvent::Known(SessionEvent::UserMessage { data, .. }) => Some(message_text(data)),
                _ => None,
            })
            .expect("user text");
        assert!(text.contains("<system-reminder>"));
        assert!(text.contains("Instructions from: AGENTS.md"));
        assert!(text.contains("Be concise."));
    }

    async fn load_and_run_resume(root: &Path, old: &str, new: &str) -> Session {
        std::fs::write(root.join("AGENTS.md"), format!("{old}\n")).unwrap();
        std::fs::write(root.join(".git"), "").unwrap();
        let sessions_dir = root.join(".sessions");
        let store = JsonlSessionStore::with_root(&sessions_dir);
        let mut session = Session::new(SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("workspace-context-resume"),
            created_at: 1,
            cwd: Some(root.to_string_lossy().into_owned()),
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        });
        session
            .append(SessionEvent::UserMessage {
                seq: 0,
                time: 0,
                data: Message {
                    id: MessageId::new("old-baseline"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text {
                        text: format!("Old wrap {old}"),
                    }],
                    source: MessageSource::AgentInstructions {
                        form: "instructions".into(),
                        baseline: Some(true),
                        baseline_identity: Some("stale-offline".into()),
                        changes: Vec::new(),
                    },
                },
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            })
            .unwrap();
        store.flush(&session).unwrap();
        std::fs::write(root.join("AGENTS.md"), format!("{new}\n")).unwrap();
        let home = root.join(".dsh-home");
        std::fs::create_dir_all(&home).expect("dsh home");
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register_stack(&mut registry);
        boot_yaml(
            &ctx,
            &instruction_yaml(&home),
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .unwrap();
        let agents = ctx.get::<AgentRegistry>("agents").expect("agents");
        let loaded = store
            .load(&SessionId::new("workspace-context-resume"))
            .unwrap();
        let handle = agents
            .resume(
                loaded,
                CreateAgentOptions {
                    session_id: SessionId::new("workspace-context-resume"),
                    cwd: Some(root.to_string_lossy().into_owned()),
                    provider: "mock".into(),
                    model: "mock".into(),
                    max_tokens: None,
                },
            )
            .unwrap();
        handle.followup(user_text("hi", "hi")).await.unwrap();
        handle.run_until_idle().await.unwrap();
        handle.lock().session.clone()
    }

    #[tokio::test]
    async fn resume_after_offline_edit_appends_new_agent_instructions() {
        let root = test_temp_dir("resume");
        let session = load_and_run_resume(
            &root,
            "Old workspace instruction.",
            "New workspace instruction after offline edit.",
        )
        .await;
        assert!(session.events().iter().any(|e| match e {
            LogEvent::Known(SessionEvent::UserMessage { data, .. }) => {
                matches!(data.source, MessageSource::AgentInstructions { .. })
                    && message_text(data).contains("New workspace instruction")
            }
            _ => false,
        }));
    }

    #[tokio::test]
    async fn unknown_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-agent-instructions'\n  config:\n    maxBytes: 65536\n    density: 3\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }

    #[test]
    fn empty_object_is_not_enough_without_max_bytes() {
        let err = resolve_config(&json!({})).unwrap_err();
        assert!(err.to_string().contains("maxBytes"));
    }
}
