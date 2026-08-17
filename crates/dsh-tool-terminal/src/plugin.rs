//! Kernel plugin `@deepseek-ai/dsh-tool-terminal`.

use std::sync::{Arc, Mutex, PoisonError};

use dsh_boot::{PLUGIN_TOOL_TERMINAL, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_system_prompt::{PromptSection, SectionText, SystemPrompt};
use dsh_terminal::TerminalSessionService;
use dsh_tools::ToolRuntime;
use serde_json::Value;

use crate::tools::register_terminal_tools;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

const GUIDANCE: &str = "Use a terminal session only when work needs persistent terminal state or interactive stdin; prefer shell/read/write/edit for bounded one-shot operations. Track every terminal session id and close sessions that no longer matter. An inferred_idle or timeout result does not prove the foreground command exited.";
const CONFIG_KEYS: &[&str] = &["enableRunInBackground", "maxResultBytes"];
const INVALID_MAX_RESULT_BYTES: &str =
    "tool-terminal: maxResultBytes must be a safe integer of at least 64";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const DEFAULT_MAX_RESULT_BYTES: usize = 256 * 1024;
const MIN_MAX_RESULT_BYTES: u64 = 64;

/// Validated `dsh-tool-terminal` configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ToolTerminalConfig {
    pub(crate) enable_run_in_background: bool,
    pub(crate) max_result_bytes: usize,
}

impl Default for ToolTerminalConfig {
    fn default() -> Self {
        Self {
            enable_run_in_background: true,
            max_result_bytes: DEFAULT_MAX_RESULT_BYTES,
        }
    }
}

/// Register YAML `@deepseek-ai/dsh-tool-terminal`.
///
/// Injects `tools`, `terminals`, and `systemPrompt`. Registers six `terminal_*`
/// tools and the `tool:pty` guidance section.
///
/// # Parameters
///
/// * `registry` - Closed plugin-name registry.
///
/// # Returns
///
/// Nothing. The YAML name is installed on `registry`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved = resolve_config(&config).map_err(setup_err)?;
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let terminals = ctx
                .inject::<Mutex<TerminalSessionService>>("terminals")
                .await?;
            let prompt = ctx.inject::<SystemPrompt>("systemPrompt").await?;
            let service = terminals
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            prompt
                .section(PromptSection {
                    name: "tool:pty".into(),
                    order: 106,
                    text: SectionText::Static(GUIDANCE.into()),
                    complete: false,
                })
                .map_err(|error| setup_err(error.to_string()))?;
            register_terminal_tools(
                &mut tools.lock().unwrap_or_else(PoisonError::into_inner),
                service,
                resolved,
            );
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_TERMINAL, setup);
}

fn resolve_config(value: &Value) -> Result<ToolTerminalConfig, String> {
    match value {
        Value::Null => Ok(ToolTerminalConfig::default()),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("ToolTerminalConfig: unknown key \"{key}\""));
                }
            }
            let mut config = ToolTerminalConfig::default();
            match map.get("enableRunInBackground") {
                None | Some(Value::Null) => {}
                Some(Value::Bool(flag)) => config.enable_run_in_background = *flag,
                Some(_) => {
                    return Err("ToolTerminalConfig.enableRunInBackground must be a boolean".into());
                }
            }
            match map.get("maxResultBytes") {
                None | Some(Value::Null) => {}
                Some(item) => {
                    let Some(number) = item.as_u64() else {
                        return Err(INVALID_MAX_RESULT_BYTES.into());
                    };
                    if !(MIN_MAX_RESULT_BYTES..=MAX_SAFE_INTEGER).contains(&number) {
                        return Err(INVALID_MAX_RESULT_BYTES.into());
                    }
                    config.max_result_bytes = usize::try_from(number)
                        .map_err(|_| INVALID_MAX_RESULT_BYTES.to_string())?;
                }
            }
            Ok(config)
        }
        _ => Err("ToolTerminalConfig: config must be an object".into()),
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex, PoisonError};

    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_session::{CallId, ContentBlock, SessionId};
    use dsh_system_prompt::{AssembleContext, SystemPrompt};
    use dsh_terminal::{
        TerminalBackend, TerminalBackendSession, TerminalBackendSpawnFuture,
        TerminalBackendSpawnSpec, TerminalError, TerminalReadRequest, TerminalReadResult,
        TerminalSendOperation, TerminalSendRead, TerminalSendRequest, TerminalSendResult,
        TerminalSessionService, TerminalSessionStatus, TerminalSignal, TerminalSignalResult,
        TerminalSpawnRequest, TerminalWaitReason,
    };
    use dsh_tools::{AbortFlag, ToolExecutionInput, ToolExecutionResult, ToolRuntime};
    use serde_json::json;

    use super::register;

    const GUIDANCE: &str = "Use a terminal session only when work needs persistent terminal state or interactive stdin; prefer shell/read/write/edit for bounded one-shot operations. Track every terminal session id and close sessions that no longer matter. An inferred_idle or timeout result does not prove the foreground command exited.";

    const STACK_YAML: &str = "\
- name: '@deepseek-ai/dsh-tools'
- name: '@deepseek-ai/dsh-system-prompt'
- name: '@deepseek-ai/dsh-terminal'
- name: pty-snapshot-backend
- name: '@deepseek-ai/dsh-tool-terminal'
";

    async fn boot_stack_yaml(tool_config: &str) -> Context {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_tools::plugin::register(&mut registry);
        dsh_system_prompt::plugin::register(&mut registry);
        dsh_terminal::register_terminal_plugins(&mut registry);
        register(&mut registry);
        let yaml = if tool_config.is_empty() {
            STACK_YAML.to_string()
        } else {
            format!(
                "\
- name: '@deepseek-ai/dsh-tools'
- name: '@deepseek-ai/dsh-system-prompt'
- name: '@deepseek-ai/dsh-terminal'
- name: pty-snapshot-backend
- name: '@deepseek-ai/dsh-tool-terminal'
  config:
    {tool_config}
"
            )
        };
        boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env())
            .await
            .expect("boot tool-terminal stack");
        ctx
    }

    async fn boot_stack() -> Context {
        boot_stack_yaml("").await
    }

    fn owner() -> SessionId {
        SessionId::new("owner-a")
    }

    async fn execute(
        tools: &Arc<Mutex<ToolRuntime>>,
        name: &str,
        arguments: serde_json::Value,
        session_id: Option<SessionId>,
    ) -> ToolExecutionResult {
        let mut runtime = tools.lock().expect("tools").clone();
        runtime
            .execute(ToolExecutionInput {
                call_id: CallId::new("c1"),
                root_call_id: None,
                name: name.into(),
                arguments,
                parent: None,
                session_id,
                signal: AbortFlag::new(),
            })
            .await
    }

    fn failure_message(result: &ToolExecutionResult) -> String {
        match result {
            ToolExecutionResult::Failure { error, .. } => error.message.clone(),
            ToolExecutionResult::Success { .. } => panic!("expected failure"),
        }
    }

    fn text_of(result: &ToolExecutionResult) -> String {
        match result.content() {
            [ContentBlock::Text { text }] => text.clone(),
            other => panic!("unexpected content {other:?}"),
        }
    }

    #[test]
    fn plugin_yaml_name_is_typescript_package_name() {
        assert_eq!(
            dsh_boot::PLUGIN_TOOL_TERMINAL,
            "@deepseek-ai/dsh-tool-terminal"
        );
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        assert!(registry.get(dsh_boot::PLUGIN_TOOL_TERMINAL).is_some());
    }

    #[tokio::test]
    async fn unknown_session_signal_error_text() {
        let ctx = boot_stack().await;
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        let result = execute(
            &tools,
            "terminal_signal",
            json!({ "sessionId": "pty-99", "signal": "SIGINT" }),
            Some(owner()),
        )
        .await;
        assert!(result.is_error());
        let message = failure_message(&result);
        assert!(message.contains("unknown PTY session pty-99"), "{message}");
    }

    #[tokio::test]
    async fn guidance_section_is_locked() {
        let ctx = boot_stack().await;
        let prompt = ctx
            .inject::<SystemPrompt>("systemPrompt")
            .await
            .expect("systemPrompt");
        let assembly = prompt
            .assemble(&AssembleContext::default())
            .expect("assemble");
        let section = assembly
            .sections
            .iter()
            .find(|section| section.name == "tool:pty")
            .expect("tool:pty section");
        assert_eq!(section.text, GUIDANCE);
    }

    #[tokio::test]
    async fn missing_session_id_fails_closed() {
        let ctx = boot_stack().await;
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        let result = execute(&tools, "terminal_list", json!({}), None).await;
        assert!(result.is_error());
        assert_eq!(
            failure_message(&result),
            "terminal tools require an initiating session"
        );
    }

    #[tokio::test]
    async fn background_send_requires_jobs() {
        let ctx = boot_stack().await;
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        let opened = execute(
            &tools,
            "terminal_open",
            json!({ "type": "shell" }),
            Some(owner()),
        )
        .await;
        assert!(!opened.is_error(), "{}", text_of(&opened));
        let result = execute(
            &tools,
            "terminal_send",
            json!({
                "sessionId": "pty-1",
                "text": "work",
                "run_in_background": true,
            }),
            Some(owner()),
        )
        .await;
        assert!(result.is_error());
        assert_eq!(
            failure_message(&result),
            "background terminal sends require @deepseek-ai/dsh-jobs and @deepseek-ai/dsh-tool-jobs"
        );
    }

    #[tokio::test]
    async fn background_disabled_rejects_forced_arg() {
        let ctx = boot_stack_yaml("enableRunInBackground: false").await;
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        let opened = execute(
            &tools,
            "terminal_open",
            json!({ "type": "shell" }),
            Some(owner()),
        )
        .await;
        assert!(!opened.is_error());
        let result = execute(
            &tools,
            "terminal_send",
            json!({
                "sessionId": "pty-1",
                "text": "work",
                "run_in_background": true,
            }),
            Some(owner()),
        )
        .await;
        assert!(result.is_error());
        assert_eq!(
            failure_message(&result),
            "background terminal sends are disabled by tool-terminal configuration"
        );
    }

    #[tokio::test]
    async fn close_closed_and_empty_list() {
        let ctx = boot_stack().await;
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        let opened = execute(
            &tools,
            "terminal_open",
            json!({ "type": "shell", "name": "main" }),
            Some(owner()),
        )
        .await;
        assert!(!opened.is_error());
        assert!(text_of(&opened).contains("started terminal session pty-1 (main)"));
        let listed = execute(&tools, "terminal_list", json!({}), Some(owner())).await;
        assert!(!listed.is_error());
        assert!(text_of(&listed).contains("pty-1 (main) [shell] running"));
        let closed = execute(
            &tools,
            "terminal_close",
            json!({ "sessionId": "pty-1" }),
            Some(owner()),
        )
        .await;
        assert!(!closed.is_error());
        assert_eq!(text_of(&closed), "closed terminal session pty-1");
        assert_eq!(closed.content().len(), 1,);
        match &closed {
            ToolExecutionResult::Success { value, .. } => {
                assert_eq!(value["outcome"], "closed");
                assert_eq!(value["sessionId"], "pty-1");
            }
            ToolExecutionResult::Failure { .. } => panic!("expected success"),
        }
        let empty = execute(&tools, "terminal_list", json!({}), Some(owner())).await;
        assert_eq!(text_of(&empty), "(no terminal sessions)");
    }

    struct HoldClose {
        started: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        release: Arc<Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
    }

    struct HoldSession {
        started: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        release: Arc<Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
    }

    impl TerminalBackendSession for HoldSession {
        fn motd(&self) -> &str {
            "held"
        }

        fn pid(&self) -> Option<i32> {
            None
        }

        fn start_send(&self, request: TerminalSendRequest) -> TerminalSendOperation {
            let result = TerminalSendResult::new(
                request.text().to_string(),
                TerminalWaitReason::Timeout,
                TerminalSessionStatus::Running,
                false,
            );
            TerminalSendOperation::new(
                Box::pin(std::future::ready(result)),
                Box::new(|| TerminalSendRead::new(String::new(), false)),
                Box::new(|| false),
            )
        }

        fn read(&self, _request: TerminalReadRequest) -> TerminalReadResult {
            TerminalReadResult::new("", 0, 0, 0, false)
        }

        fn signal(
            &self,
            _signal: TerminalSignal,
        ) -> Pin<Box<dyn Future<Output = TerminalSignalResult> + Send>> {
            Box::pin(std::future::ready(TerminalSignalResult::new(true, 1)))
        }

        fn status(&self) -> TerminalSessionStatus {
            TerminalSessionStatus::Running
        }

        fn close(
            &self,
            _reason: &str,
        ) -> Pin<Box<dyn Future<Output = Result<(), TerminalError>> + Send>> {
            let started = self
                .started
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            let release = self
                .release
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            Box::pin(async move {
                if let Some(tx) = started {
                    let _ = tx.send(());
                }
                if let Some(rx) = release {
                    let _ = rx.await;
                }
                Ok(())
            })
        }
    }

    impl TerminalBackend for HoldClose {
        fn spawn(&self, _spec: TerminalBackendSpawnSpec) -> TerminalBackendSpawnFuture {
            let started = Arc::clone(&self.started);
            let release = Arc::clone(&self.release);
            Box::pin(async move {
                Ok(Box::new(HoldSession { started, release }) as Box<dyn TerminalBackendSession>)
            })
        }
    }

    #[tokio::test]
    async fn close_already_closing() {
        let ctx = boot_stack().await;
        let terminals = ctx
            .inject::<Mutex<TerminalSessionService>>("terminals")
            .await
            .expect("terminals");
        let service = terminals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        service
            .register_backend(
                "held",
                Arc::new(HoldClose {
                    started: Arc::new(Mutex::new(Some(started_tx))),
                    release: Arc::new(Mutex::new(Some(release_rx))),
                }),
            )
            .expect("register held");
        let spawned = service
            .spawn(owner(), TerminalSpawnRequest::new("held"))
            .await
            .expect("spawn");
        let first = tokio::spawn({
            let service = service.clone();
            let id = spawned.session_id().clone();
            async move { service.kill(&owner(), &id, "model request").await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), started_rx)
            .await
            .expect("first closer called backend close")
            .expect("close started");
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        let second_fut = execute(
            &tools,
            "terminal_close",
            json!({ "sessionId": spawned.session_id().as_str() }),
            Some(owner()),
        );
        tokio::pin!(second_fut);
        for _ in 0..32 {
            tokio::select! {
                biased;
                result = &mut second_fut => panic!("close finished before release: {result:?}"),
                () = tokio::task::yield_now() => {}
            }
        }
        release_tx.send(()).expect("release close");
        assert!(first.await.expect("join").expect("first kill"));
        let second = second_fut.await;
        assert!(!second.is_error());
        assert_eq!(
            text_of(&second),
            format!(
                "terminal session {} was already closing",
                spawned.session_id()
            )
        );
        match &second {
            ToolExecutionResult::Success { value, .. } => {
                assert_eq!(value["outcome"], "already-closing");
            }
            ToolExecutionResult::Failure { .. } => panic!("expected success"),
        }
    }

    #[tokio::test]
    async fn open_send_read_signal_match_snapshot_backend() {
        let ctx = boot_stack().await;
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        let opened = execute(
            &tools,
            "terminal_open",
            json!({ "type": "shell" }),
            Some(owner()),
        )
        .await;
        assert!(!opened.is_error(), "{}", text_of(&opened));
        assert_eq!(
            text_of(&opened),
            "started terminal session pty-1 [type: shell]\ndsh> "
        );
        match &opened {
            ToolExecutionResult::Success { value, .. } => {
                assert_eq!(value["sessionId"], "pty-1");
                assert_eq!(value["type"], "shell");
                assert_eq!(value["motd"], "dsh> ");
                assert_eq!(value["status"]["kind"], "running");
            }
            ToolExecutionResult::Failure { .. } => panic!("expected success"),
        }
        let sent = execute(
            &tools,
            "terminal_send",
            json!({ "sessionId": "pty-1", "text": "hi" }),
            Some(owner()),
        )
        .await;
        assert!(!sent.is_error(), "{}", text_of(&sent));
        assert!(text_of(&sent).contains("PTY_OK"));
        assert!(text_of(&sent).contains("[wait: stdin_read]"));
        match &sent {
            ToolExecutionResult::Success { value, .. } => {
                assert_eq!(value["kind"], "foreground");
                assert_eq!(value["waitReason"], "stdin_read");
                assert_eq!(value["truncated"], false);
            }
            ToolExecutionResult::Failure { .. } => panic!("expected success"),
        }
        let read = execute(
            &tools,
            "terminal_read",
            json!({ "sessionId": "pty-1" }),
            Some(owner()),
        )
        .await;
        assert!(!read.is_error(), "{}", text_of(&read));
        assert!(text_of(&read).contains("dsh>"));
        let signaled = execute(
            &tools,
            "terminal_signal",
            json!({ "sessionId": "pty-1", "signal": "SIGINT" }),
            Some(owner()),
        )
        .await;
        assert!(!signaled.is_error(), "{}", text_of(&signaled));
        assert_eq!(
            text_of(&signaled),
            "delivered SIGINT to foreground process group 1"
        );
        match &signaled {
            ToolExecutionResult::Success { value, .. } => {
                assert_eq!(value["delivered"], true);
                assert_eq!(value["targetPgid"], 1);
            }
            ToolExecutionResult::Failure { .. } => panic!("expected success"),
        }
    }

    #[tokio::test]
    async fn empty_type_and_session_id_match_typescript() {
        let ctx = boot_stack().await;
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        let opened = execute(
            &tools,
            "terminal_open",
            json!({ "type": "" }),
            Some(owner()),
        )
        .await;
        assert_eq!(failure_message(&opened), "type must be a non-empty string");
        let signaled = execute(
            &tools,
            "terminal_signal",
            json!({ "sessionId": "", "signal": "SIGINT" }),
            Some(owner()),
        )
        .await;
        assert_eq!(
            failure_message(&signaled),
            "sessionId must be a non-empty string"
        );
    }

    #[tokio::test]
    async fn unknown_config_key_fails_plugin_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_tools::plugin::register(&mut registry);
        dsh_system_prompt::plugin::register(&mut registry);
        dsh_terminal::register_terminal_plugins(&mut registry);
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "\
- name: '@deepseek-ai/dsh-tools'
- name: '@deepseek-ai/dsh-system-prompt'
- name: '@deepseek-ai/dsh-terminal'
- name: pty-snapshot-backend
- name: '@deepseek-ai/dsh-tool-terminal'
  config:
    spawn: true
",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("unknown key"), "{err}");
    }

    #[tokio::test]
    async fn max_result_bytes_below_64_fails_load() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_tools::plugin::register(&mut registry);
        dsh_system_prompt::plugin::register(&mut registry);
        dsh_terminal::register_terminal_plugins(&mut registry);
        register(&mut registry);
        let err = boot_yaml(
            &ctx,
            "\
- name: '@deepseek-ai/dsh-tools'
- name: '@deepseek-ai/dsh-system-prompt'
- name: '@deepseek-ai/dsh-terminal'
- name: pty-snapshot-backend
- name: '@deepseek-ai/dsh-tool-terminal'
  config:
    maxResultBytes: 63
",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("tool-terminal: maxResultBytes must be a safe integer of at least 64"),
            "{err}"
        );
    }
}
