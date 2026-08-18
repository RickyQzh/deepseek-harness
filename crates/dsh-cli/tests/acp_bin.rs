//! Spawn `dsh --profile acp` and probe initialize-then-EOF.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::SystemTime;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

fn test_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[tokio::test]
async fn dsh_acp_bin_initialize_then_eof_prints_acp_agent_info() {
    let bin = env!("CARGO_BIN_EXE_dsh");
    let root = test_temp_dir("dsh-cli-acp");
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).expect("sessions");
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("home");

    let mut child = Command::new(bin)
        .args(["--profile", "acp"])
        .env("DSH_SESSION_ROOT", &sessions)
        .env("DSH_HOME", &home)
        .env("DSH_CWD", &root)
        .env_remove("DSH_CORDIS_CONFIG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn dsh acp");

    let mut stdin = child.stdin.take().expect("stdin");
    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}"#,
        )
        .await
        .expect("write initialize");
    stdin.write_all(b"\n").await.expect("newline");
    drop(stdin);

    let output = child.wait_with_output().await.expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "exit {:?}: stdout={stdout:?} stderr={stderr:?}",
        output.status
    );
    assert!(
        stdout.contains("deepseek-harness-acp"),
        "missing acp agentInfo: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !stdout.contains("deepseek-harness-sdk-runtime"),
        "sdk runtime name leaked: {stdout:?}"
    );
}
