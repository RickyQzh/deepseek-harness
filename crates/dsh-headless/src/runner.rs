//! Kernel plugin `headless-runner`.

use std::sync::Arc;

use dsh_agent::{AgentRegistry, CreateAgentOptions};
use dsh_boot::{PLUGIN_HEADLESS_RUNNER, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_session::{
    ContentBlock, Message, MessageId, MessageRole, MessageSource, SessionId, TurnEndReason,
};
use dsh_session_persist::JsonlSessionStore;

use crate::error::HeadlessError;
use crate::io::{AppExit, HeadlessIo, HeadlessStartup};
use crate::summarize::summarize;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

fn mint_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{}-{}", std::process::id(), nanos)
}

fn fail(io: &HeadlessIo, exit: &AppExit, error: &HeadlessError) {
    io.write_stderr(&format!("dsh: {error}\n"));
    exit.exit(1);
}

async fn run(
    task: String,
    agents: Arc<AgentRegistry>,
    sessions: Arc<JsonlSessionStore>,
    io: HeadlessIo,
    exit: Arc<AppExit>,
) {
    match run_inner(task, agents, sessions, &io).await {
        Ok(code) => exit.exit(code),
        Err(error) => fail(&io, &exit, &error),
    }
}

async fn run_inner(
    task: String,
    agents: Arc<AgentRegistry>,
    sessions: Arc<JsonlSessionStore>,
    io: &HeadlessIo,
) -> Result<i32, HeadlessError> {
    let providers = agents.list_providers();
    let provider = providers.first().cloned().ok_or_else(|| {
        HeadlessError::Kernel(KernelError::SetupFailed(
            "no LLM provider registered".into(),
        ))
    })?;
    let model = if provider == "deepseek-official" {
        "deepseek-v4-flash".to_string()
    } else {
        provider.clone()
    };
    let session_id = SessionId::new(mint_id("session"));
    let cwd = std::env::var("DSH_CWD").ok().or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    });
    let handle = agents.create(CreateAgentOptions {
        session_id,
        cwd,
        provider,
        model,
        max_tokens: None,
    })?;
    let key = handle.id().as_str().to_string();
    agents.when_idle(&key).await?;
    let first_seq = {
        let guard = handle.lock().await;
        guard.session.events().len() as u64
    };
    let message = Message {
        id: MessageId::new(mint_id("msg")),
        role: MessageRole::User,
        content: vec![ContentBlock::Text { text: task }],
        source: MessageSource::User,
    };
    agents.followup(&key, message).await?;
    agents.when_idle(&key).await?;
    {
        let guard = handle.lock().await;
        sessions.flush(&guard.session)?;
        let outcome = summarize(guard.session.events(), first_seq);
        io.write_stdout(&format!("{}\n", outcome.text));
        if let Some(TurnEndReason::Error { error }) = &outcome.reason {
            io.write_stderr(&format!("dsh: {}: {}\n", error.code, error.message));
        }
        let code = match &outcome.reason {
            Some(TurnEndReason::Completed) => 0,
            _ => 1,
        };
        Ok(code)
    }
}

/// Drive one task after `headlessStartup` is provided.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config| {
        Box::pin(async move {
            let startup = ctx.inject::<HeadlessStartup>("headlessStartup").await?;
            let agents = ctx.inject::<AgentRegistry>("agents").await?;
            let sessions = ctx.inject::<JsonlSessionStore>("sessions").await?;
            let exit = ctx
                .get::<AppExit>("appExit")
                .ok_or_else(|| setup_err(HeadlessError::MissingAppExit.to_string()))?;
            let io = ctx
                .get::<HeadlessIo>("headlessIo")
                .map(|arc| (*arc).clone())
                .unwrap_or_else(HeadlessIo::stdio);
            let task = startup.task().to_string();
            drop(tokio::spawn(run(task, agents, sessions, io, exit)));
            Ok(())
        })
    });
    registry.register(PLUGIN_HEADLESS_RUNNER, setup);
}

#[cfg(test)]
mod tests {
    use crate::{AppExit, CmdlineArgs, HeadlessIo, MINIMAL_YAML, register_headless_plugins};
    use dsh_agent::{register_execution_plugins, register_spine_plugins};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    fn test_temp_dir(prefix: &str) -> std::path::PathBuf {
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

    #[tokio::test]
    async fn mock_llm_prints_last_text_and_exits_0_on_completed() {
        let root = test_temp_dir("dsh-headless");
        let previous = std::env::var("DSH_SESSION_ROOT").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", root.as_os_str());
        }
        let ctx = Context::new();
        let (exit, rx) = AppExit::pair();
        ctx.provide("appExit", exit).unwrap();
        ctx.provide("cmdlineArgs", CmdlineArgs::new(vec!["unused task".into()]))
            .unwrap();
        let io = HeadlessIo::capture();
        ctx.provide("headlessIo", io.clone()).unwrap();
        let mut registry = PluginRegistry::new();
        register_spine_plugins(&mut registry);
        register_execution_plugins(&mut registry);
        register_headless_plugins(&mut registry);
        boot_yaml(
            &ctx,
            MINIMAL_YAML,
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .unwrap();
        let code = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .expect("appExit timed out")
            .expect("appExit dropped");
        assert_eq!(code, 0, "stderr={}", io.stderr_text());
        assert_eq!(io.stdout_text(), "headless-ok\n");
        assert_eq!(io.stderr_text(), "");
        match previous {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
    }
}
