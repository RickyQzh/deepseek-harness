//! Reconnect supervisor tests against the crate-local stdio fixture.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
use dsh_kernel::Context;
use dsh_mcp_client::public_tool_name;
use dsh_session::{CallId, ContentBlock};
use dsh_tools::{AbortFlag, ToolExecutionInput, ToolExecutionResult, ToolRuntime};
use serde_json::{Value, json};

fn fixture_command_yaml() -> String {
    serde_json::to_string(env!("CARGO_BIN_EXE_dsh-mcp-fixture")).expect("command json string")
}

fn mcp_reconnect_yaml(server_name: &str, reconnect_body: &str) -> String {
    let command = fixture_command_yaml();
    format!(
        "- name: '@deepseek-ai/dsh-tools'\n- name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    transport: stdio\n    serverName: {server_name}\n    command: {command}\n    failOnStartupError: true\n    reconnect:\n{reconnect_body}"
    )
}

fn fast_reconnect_body(enabled: bool) -> String {
    format!(
        "      enabled: {enabled}\n      initialDelayMs: 20\n      maxDelayMs: 40\n      maxAttempts: 2\n"
    )
}

fn kill_remaining_fixture_children() {
    let me = std::process::id();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in dir.flatten() {
        let pid: u32 = match entry.file_name().to_str().and_then(|s| s.parse().ok()) {
            Some(pid) => pid,
            None => continue,
        };
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        if comm.trim() != "dsh-mcp-fixture" {
            continue;
        }
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
        let mut ppid_match = false;
        for line in status.lines() {
            let Some(rest) = line.strip_prefix("PPid:") else {
                continue;
            };
            ppid_match = rest.trim().parse::<u32>().ok() == Some(me);
            break;
        }
        if !ppid_match {
            continue;
        }
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
    }
}

fn fixture_child_pids() -> std::collections::HashSet<u32> {
    let me = std::process::id();
    let mut pids = std::collections::HashSet::new();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return pids;
    };
    for entry in dir.flatten() {
        let pid: u32 = match entry.file_name().to_str().and_then(|s| s.parse().ok()) {
            Some(pid) => pid,
            None => continue,
        };
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        if comm.trim() != "dsh-mcp-fixture" {
            continue;
        }
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
        for line in status.lines() {
            let Some(rest) = line.strip_prefix("PPid:") else {
                continue;
            };
            if rest.trim().parse::<u32>().ok() == Some(me) {
                pids.insert(pid);
            }
            break;
        }
    }
    pids
}

async fn boot(
    yaml: &str,
) -> (
    tokio::sync::MutexGuard<'static, ()>,
    Context,
    Arc<Mutex<ToolRuntime>>,
) {
    let lock = dsh_mcp_client::lock_stdio_fixture_tests().await;
    assert!(
        !yaml.contains("!!js"),
        "YAML added this item must not contain !!js"
    );
    let ctx = Context::new();
    let mut registry = PluginRegistry::new();
    dsh_tools::plugin::register(&mut registry);
    dsh_mcp_client::register(&mut registry);
    boot_yaml(&ctx, yaml, &[], &registry, &process_interpolate_env())
        .await
        .map(|_| ())
        .expect("boot mcp stdio");
    let tools = ctx
        .inject::<Mutex<ToolRuntime>>("tools")
        .await
        .expect("tools");
    (lock, ctx, tools)
}

fn execute_input(name: &str, arguments: Value) -> ToolExecutionInput {
    ToolExecutionInput {
        call_id: CallId::new("c1"),
        root_call_id: None,
        name: name.into(),
        arguments,
        parent: None,
        session_id: None,
        signal: AbortFlag::new(),
    }
}

async fn wait_until_add_errors(tools: &Arc<Mutex<ToolRuntime>>, add: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let result = execute_tool(tools, add, json!({"a": 2, "b": 3})).await;
            if result.is_error() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("add must fail after the child exits");
}

async fn wait_until_add_succeeds(
    tools: &Arc<Mutex<ToolRuntime>>,
    add: &str,
) -> ToolExecutionResult {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let result = execute_tool(tools, add, json!({"a": 2, "b": 3})).await;
            if !result.is_error() {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("add recovered after crash")
}

async fn execute_tool(
    tools: &Arc<Mutex<ToolRuntime>>,
    name: &str,
    arguments: Value,
) -> ToolExecutionResult {
    let mut runtime = tools.lock().expect("tools").clone();
    runtime.execute(execute_input(name, arguments)).await
}

fn text_of(result: &ToolExecutionResult) -> &str {
    match result.content() {
        [ContentBlock::Text { text }] => text.as_str(),
        other => panic!("unexpected content {other:?}"),
    }
}

fn registered(tools: &Arc<Mutex<ToolRuntime>>, name: &str) -> bool {
    tools
        .lock()
        .expect("tools")
        .registered_names()
        .iter()
        .any(|n| n == name)
}

#[test]
fn resolve_reconnect_policy_rejects_inverted_delays() {
    let err = dsh_mcp_client::resolve_reconnect_policy(
        Some(&json!({
            "initialDelayMs": 100,
            "maxDelayMs": 5
        })),
        "mcp-client(srv): reconnect",
    )
    .expect_err("inverted delays must fail");
    assert_eq!(
        err,
        "mcp-client(srv): reconnect.initialDelayMs must be less than or equal to maxDelayMs"
    );
}

#[tokio::test]
async fn stdio_crash_reconnects_and_add_works() {
    let yaml = mcp_reconnect_yaml("crashy", &fast_reconnect_body(true));
    let (_lock, ctx, tools) = boot(&yaml).await;
    let crash = public_tool_name("crashy", "crash");
    let add = public_tool_name("crashy", "add");
    let crashed = execute_tool(&tools, &crash, json!({})).await;
    assert!(
        !crashed.is_error(),
        "crash must reply before exit: {}",
        text_of(&crashed)
    );
    assert_eq!(text_of(&crashed), "crashing");
    wait_until_add_errors(&tools, &add).await;
    let recovered = wait_until_add_succeeds(&tools, &add).await;
    assert_eq!(text_of(&recovered), "5");
    ctx.dispose().await;
    kill_remaining_fixture_children();
}

#[tokio::test]
async fn reconnect_disabled_does_not_respawn() {
    let yaml = mcp_reconnect_yaml("quiet", &fast_reconnect_body(false));
    let (_lock, ctx, tools) = boot(&yaml).await;
    let before = fixture_child_pids();
    let crash = public_tool_name("quiet", "crash");
    let add = public_tool_name("quiet", "add");
    let crashed = execute_tool(&tools, &crash, json!({})).await;
    assert!(!crashed.is_error(), "{}", text_of(&crashed));
    wait_until_add_errors(&tools, &add).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let spawned: Vec<u32> = fixture_child_pids().difference(&before).copied().collect();
    assert!(
        spawned.is_empty(),
        "disabled reconnect must not spawn a replacement child, new pids: {spawned:?}"
    );
    let after = execute_tool(&tools, &add, json!({"a": 2, "b": 3})).await;
    assert!(
        after.is_error(),
        "add must not talk to a replacement process: {}",
        text_of(&after)
    );
    ctx.dispose().await;
    kill_remaining_fixture_children();
}

#[tokio::test]
async fn exhausted_attempts_unregister_tools() {
    let yaml = mcp_reconnect_yaml("cap", &fast_reconnect_body(true));
    let (_lock, ctx, tools) = boot(&yaml).await;
    let crash = public_tool_name("cap", "crash");
    let add = public_tool_name("cap", "add");
    assert!(registered(&tools, &add));
    assert!(registered(&tools, &crash));

    async fn crash_then_wait_add(tools: &Arc<Mutex<ToolRuntime>>, crash: &str, add: &str) {
        let crashed = execute_tool(tools, crash, json!({})).await;
        assert!(!crashed.is_error(), "{}", text_of(&crashed));
        wait_until_add_errors(tools, add).await;
        let recovered = wait_until_add_succeeds(tools, add).await;
        assert_eq!(text_of(&recovered), "5");
    }

    crash_then_wait_add(&tools, &crash, &add).await;
    crash_then_wait_add(&tools, &crash, &add).await;
    let crashed = execute_tool(&tools, &crash, json!({})).await;
    assert!(!crashed.is_error(), "{}", text_of(&crashed));
    wait_until_add_errors(&tools, &add).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !registered(&tools, &add) && !registered(&tools, &crash) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned public names unregistered after maxAttempts");
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        !registered(&tools, &add) && !registered(&tools, &crash),
        "supervisor must stay stopped after exhaustion"
    );
    ctx.dispose().await;
    kill_remaining_fixture_children();
}
