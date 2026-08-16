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
    resume_session_id: Option<String>,
    agents: Arc<AgentRegistry>,
    sessions: Arc<JsonlSessionStore>,
    io: HeadlessIo,
    exit: Arc<AppExit>,
) {
    match run_inner(task, resume_session_id, agents, sessions, &io).await {
        Ok(code) => exit.exit(code),
        Err(error) => fail(&io, &exit, &error),
    }
}

async fn run_inner(
    task: String,
    resume_session_id: Option<String>,
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
    let cwd = std::env::var("DSH_CWD").ok().or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    });
    let handle = if let Some(resume_id) = resume_session_id {
        let session = sessions.load(&SessionId::new(resume_id))?;
        let session_id = session.id().clone();
        let cwd = session.header().cwd.clone().or(cwd);
        agents.resume(
            session,
            CreateAgentOptions {
                session_id,
                cwd,
                provider,
                model,
                max_tokens: None,
            },
        )?
    } else {
        let session_id = SessionId::new(mint_id("session"));
        agents.create(CreateAgentOptions {
            session_id,
            cwd,
            provider,
            model,
            max_tokens: None,
        })?
    };
    let key = handle.id().as_str().to_string();
    agents.when_idle(&key).await?;
    let first_seq = {
        let guard = handle.lock();
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
        let guard = handle.lock();
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
            let resume_session_id = startup.resume_session_id().map(str::to_string);
            drop(tokio::spawn(run(
                task,
                resume_session_id,
                agents,
                sessions,
                io,
                exit,
            )));
            Ok(())
        })
    });
    registry.register(PLUGIN_HEADLESS_RUNNER, setup);
}

#[cfg(test)]
mod tests {
    use crate::{
        AppExit, BASE_YAML, CmdlineArgs, HeadlessIo, MINIMAL_YAML, register_headless_plugins,
    };
    use dsh_agent::{register_execution_plugins, register_spine_plugins};
    use dsh_base::register_base_plugins;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_session::{SESSION_FORMAT_VERSION, Session, SessionHeader, SessionId};
    use dsh_session_persist::JsonlSessionStore;
    use std::sync::Mutex;

    static SESSION_ROOT_LOCK: Mutex<()> = Mutex::new(());

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

    fn restore_session_root(previous: Option<String>) {
        match previous {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
    }

    #[tokio::test]
    async fn mock_llm_prints_last_text_and_exits_0_on_completed() {
        let _guard = SESSION_ROOT_LOCK.lock().expect("session root");
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
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn resume_session_id_loads_existing_jsonl_then_followup() {
        let _guard = SESSION_ROOT_LOCK.lock().expect("session root");
        let root = test_temp_dir("dsh-headless-resume");
        let previous = std::env::var("DSH_SESSION_ROOT").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", root.as_os_str());
        }
        let store = JsonlSessionStore::with_root(&root);
        let session = Session::new(SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("workspace-context-resume"),
            created_at: 1,
            cwd: Some("/work".into()),
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        });
        store.flush(&session).unwrap();
        let yaml = MINIMAL_YAML.replace(
            "- name: headless-startup\n",
            "- name: headless-startup\n  config:\n    resumeSessionId: workspace-context-resume\n",
        );
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
        boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env())
            .await
            .unwrap();
        let code = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .expect("appExit timed out")
            .expect("appExit dropped");
        assert_eq!(code, 0, "stderr={}", io.stderr_text());
        assert_eq!(io.stdout_text(), "headless-ok\n");
        let loaded = store
            .load(&SessionId::new("workspace-context-resume"))
            .unwrap();
        assert!(
            loaded.events().len() > session.events().len(),
            "resume must append the cmdline followup onto the seeded session"
        );
        restore_session_root(previous);
    }

    #[tokio::test]
    async fn base_yaml_mock_llm_prints_base_ok_and_exits_0() {
        let _guard = SESSION_ROOT_LOCK.lock().expect("session root");
        let root = test_temp_dir("dsh-headless-base");
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
        register_base_plugins(&mut registry);
        register_headless_plugins(&mut registry);
        let boot = boot_yaml(&ctx, BASE_YAML, &[], &registry, &process_interpolate_env()).await;
        if let Err(error) = boot {
            restore_session_root(previous);
            panic!("{error}");
        }
        let timed = tokio::time::timeout(std::time::Duration::from_secs(30), rx).await;
        let stdout = io.stdout_text();
        let stderr = io.stderr_text();
        restore_session_root(previous);
        let code = timed.expect("appExit timed out").expect("appExit dropped");
        assert_eq!(code, 0, "stderr={stderr}");
        assert_eq!(stdout, "base-ok\n");
        assert_eq!(stderr, "");
    }
}
