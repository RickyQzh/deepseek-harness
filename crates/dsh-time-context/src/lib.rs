//! Optional pre-step time-context injection for the Rust host.

mod plugin;

pub use plugin::{ConfigError, TimeContextConfig, install_time_context, register, resolve_config};

#[cfg(test)]
mod tests {
    use super::{TimeContextConfig, install_time_context, register, resolve_config};
    use crate::plugin::{
        apply_injection, format_duration, format_utc_timestamp, view_from_session,
    };
    use dsh_agent_loop::{LoopAgent, LoopOptions, PreStepDecision};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response};
    use dsh_session::{
        ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource,
        SESSION_FORMAT_VERSION, Session, SessionEvent, SessionHeader, SessionId, StepBoundaryData,
        SurfaceOp, TurnStartData,
    };
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

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

    fn claimed() -> Vec<Message> {
        vec![user_text("hi", "hi")]
    }

    fn session_header(id: &str) -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new(id),
            created_at: 1,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        }
    }

    fn session_with_turn(turn: u64) -> Session {
        let mut session = Session::new(session_header("time-unit"));
        session
            .append(SessionEvent::TurnStart {
                seq: 0,
                time: 0,
                data: TurnStartData { turn },
                ignorable: None,
            })
            .unwrap();
        session
    }

    fn plugin_user(seq: u64, time: i64) -> SessionEvent {
        SessionEvent::UserMessage {
            seq,
            time,
            data: Message {
                id: MessageId::new(format!("t{seq}")),
                role: MessageRole::User,
                content: vec![ContentBlock::Text {
                    text: "prior".into(),
                }],
                source: MessageSource::Plugin {
                    plugin: "time-context".into(),
                    form: None,
                    sections: Vec::new(),
                    summary: None,
                    compaction_id: None,
                    source_command_id: None,
                },
            },
            surface_op: Some(SurfaceOp::Append),
            source_event_seqs: None,
            ignorable: None,
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

    fn enter_texts(decision: PreStepDecision) -> Vec<String> {
        match decision {
            PreStepDecision::Enter { messages } => messages.iter().map(message_text).collect(),
            PreStepDecision::Reject => Vec::new(),
        }
    }

    fn agent_on(ctx: Context) -> LoopAgent {
        let llm = Arc::new(Mutex::new(LlmRuntime::new()));
        let adapter = Arc::new(MockAdapter::new(vec![MockScript::Chunks(text_response(
            "ok",
        ))]));
        llm.lock().expect("llm").register_adapter("mock", adapter);
        LoopAgent::new(
            ctx,
            Session::new(session_header("time-context")),
            LoopOptions {
                provider: "mock".into(),
                model: "mock".into(),
                max_tokens: None,
                max_parallel_tool_calls: 10,
            },
            Arc::new(Mutex::new(ToolRuntime::new(ToolPresentationMode::Native))),
            SystemPrompt::new(SystemPromptConfig {
                include_runtime_context: false,
                include_harness_identity: false,
                ..SystemPromptConfig::default()
            })
            .unwrap(),
            llm,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn injects_time_context_on_first_step() {
        let ctx = Context::new();
        install_time_context(
            &ctx,
            TimeContextConfig {
                time_zone: Some("UTC".into()),
                refresh_interval_ms: Some(0),
            },
        );
        let mut agent = agent_on(ctx);
        agent.followup(user_text("hi", "hi")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert!(agent.session.events().iter().any(|e| match e {
            LogEvent::Known(SessionEvent::UserMessage { data, .. }) => matches!(
                &data.source,
                MessageSource::Plugin { plugin, .. } if plugin == "time-context"
            ),
            _ => false,
        }));
    }

    #[test]
    fn format_duration_matches_typescript() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(500), "0s");
        assert_eq!(format_duration(1_000), "1s");
        assert_eq!(format_duration(61_000), "1m 1s");
        assert_eq!(format_duration(3_661_000), "1h 1m 1s");
        assert_eq!(format_duration(90_061_000), "1d 1h 1m 1s");
    }

    #[test]
    fn format_utc_timestamp_is_fixed_width() {
        assert_eq!(format_utc_timestamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc_timestamp(61_000), "1970-01-01 00:01:01 UTC");
        assert_eq!(
            format_utc_timestamp(1_709_164_800_000),
            "2024-02-29 00:00:00 UTC"
        );
        assert_eq!(
            format_utc_timestamp(1_786_838_400_000),
            "2026-08-16 00:00:00 UTC"
        );
    }

    #[test]
    fn skips_reject_and_empty_enter() {
        let session = session_with_turn(1);
        let view = view_from_session(&session);
        let config = TimeContextConfig::default();
        assert!(matches!(
            apply_injection(PreStepDecision::Reject, &view, 0, &config),
            PreStepDecision::Reject
        ));
        let empty = apply_injection(
            PreStepDecision::Enter {
                messages: Vec::new(),
            },
            &view,
            0,
            &config,
        );
        assert!(matches!(
            empty,
            PreStepDecision::Enter { ref messages } if messages.is_empty()
        ));
    }

    #[test]
    fn first_step_text_uses_model_visible_baseline() {
        let session = session_with_turn(1);
        let view = view_from_session(&session);
        let decision = apply_injection(
            PreStepDecision::Enter {
                messages: claimed(),
            },
            &view,
            0,
            &TimeContextConfig::default(),
        );
        let texts = enter_texts(decision);
        assert_eq!(texts[1], "hi");
        assert_eq!(
            texts[0],
            "Time sampled while preparing turn 1, step 1: 1970-01-01 00:00:00 UTC\n\
             Elapsed since the preceding model-visible message: unavailable."
        );
    }

    #[test]
    fn later_step_uses_step_context_baseline_and_duration() {
        let mut session = session_with_turn(3);
        session
            .append(SessionEvent::StepStart {
                seq: 1,
                time: 1,
                data: StepBoundaryData { turn: 3, step: 1 },
                ignorable: None,
            })
            .unwrap();
        session.append(plugin_user(2, 0)).unwrap();
        let view = view_from_session(&session);
        let decision = apply_injection(
            PreStepDecision::Enter {
                messages: claimed(),
            },
            &view,
            61_000,
            &TimeContextConfig::default(),
        );
        let texts = enter_texts(decision);
        assert_eq!(
            texts[0],
            "Time sampled while preparing turn 3, step 2: 1970-01-01 00:01:01 UTC\n\
             Elapsed since the preceding step context: 1m 1s."
        );
    }

    #[test]
    fn positive_interval_skips_until_threshold() {
        let mut session = session_with_turn(1);
        session.append(plugin_user(1, 1_000)).unwrap();
        let view = view_from_session(&session);
        let config = TimeContextConfig {
            time_zone: None,
            refresh_interval_ms: Some(1_000),
        };
        let skipped = apply_injection(
            PreStepDecision::Enter {
                messages: claimed(),
            },
            &view,
            1_999,
            &config,
        );
        assert_eq!(enter_texts(skipped).len(), 1);
        let injected = apply_injection(
            PreStepDecision::Enter {
                messages: claimed(),
            },
            &view,
            2_000,
            &config,
        );
        assert_eq!(enter_texts(injected).len(), 2);
    }

    #[test]
    fn unknown_config_key_fails_plugin_load() {
        let err = resolve_config(&json!({"density": 3})).unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }

    #[test]
    fn negative_refresh_interval_fails_with_refresh_interval_ms() {
        let err = resolve_config(&json!({"refreshIntervalMs": -1})).unwrap_err();
        assert!(err.to_string().contains("refreshIntervalMs"));
    }

    #[tokio::test]
    async fn unknown_yaml_key_fails_boot() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-time-context'\n  config:\n    density: 3\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown key"));
    }
}
