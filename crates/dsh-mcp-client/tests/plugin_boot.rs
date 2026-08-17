//! YAML plugin load tests for `@deepseek-ai/dsh-mcp-client`.

use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
use dsh_kernel::Context;

fn mcp_stdio_row(server_name: &str, command: &str) -> String {
    let command = serde_json::to_string(command).expect("command json string");
    format!(
        "- name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    transport: stdio\n    serverName: {server_name}\n    command: {command}\n"
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

#[tokio::test]
async fn duplicate_server_name_fails_second_instance() {
    let command = env!("CARGO_BIN_EXE_dsh-mcp-fixture");
    let yaml = format!(
        "- name: '@deepseek-ai/dsh-tools'\n{}{}",
        mcp_stdio_row("dup", command),
        mcp_stdio_row("dup", command)
    );
    assert!(
        !yaml.contains("!!js"),
        "YAML added this item must not contain !!js"
    );
    let ctx = Context::new();
    let mut registry = PluginRegistry::new();
    dsh_tools::plugin::register(&mut registry);
    dsh_mcp_client::register(&mut registry);
    let err = boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env())
        .await
        .map(|_| ())
        .expect_err("second instance must fail");
    ctx.dispose().await;
    kill_remaining_fixture_children();
    let text = err.to_string();
    assert!(
        text.contains(
            "mcp-client: serverName \"dup\" is already in use by another mcp-client instance — pick a unique serverName in cordis.yml"
        ),
        "duplicate serverName: {text}"
    );
}

#[tokio::test]
async fn streamable_http_fails_load() {
    let yaml = "\
- name: '@deepseek-ai/dsh-mcp-client'
  config:
    transport: streamable-http
    serverName: web
    command: /bin/true
";
    assert!(!yaml.contains("!!js"));
    let ctx = Context::new();
    let mut registry = PluginRegistry::new();
    dsh_mcp_client::register(&mut registry);
    let err = boot_yaml(&ctx, yaml, &[], &registry, &process_interpolate_env())
        .await
        .map(|_| ())
        .expect_err("streamable-http must fail load");
    ctx.dispose().await;
    let text = err.to_string();
    assert!(
        text.contains("mcp-client: streamable-http is not supported in Phase 8 item 2"),
        "http load error: {text}"
    );
}
