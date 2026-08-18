//! Spawn the `dsh` web binary and probe `host.describe`.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

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

fn ready_url(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("dsh web: ")
            .filter(|url| url.starts_with("http://127.0.0.1:"))
            .map(str::to_string)
    })
}

async fn drain_lines<R>(reader: BufReader<R>, tx: mpsc::UnboundedSender<String>)
where
    R: AsyncRead + Unpin,
{
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if tx.send(line).is_err() {
            break;
        }
    }
}

#[tokio::test]
async fn dsh_web_bin_prints_ready_and_host_describe() {
    let bin = env!("CARGO_BIN_EXE_dsh");
    let root = test_temp_dir("dsh-cli-web");
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).expect("dist");
    std::fs::write(
        dist.join("index.html"),
        "<html><head></head><body>ok</body></html>",
    )
    .expect("index");
    let clients = root.join("client");
    std::fs::create_dir_all(&clients).expect("client packages");
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).expect("sessions");
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("home");

    let mut child = Command::new(bin)
        .args(["web", "--port", "0"])
        .env("DSH_SESSION_ROOT", &sessions)
        .env("DSH_HOME", &home)
        .env("DSH_CWD", &root)
        .env("DSH_WEB_DIST", &dist)
        .env("DSH_CLIENT_PACKAGES", &clients)
        .env_remove("DSH_CORDIS_CONFIG")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn dsh web");

    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(drain_lines(BufReader::new(stdout), tx.clone()));
    tokio::spawn(drain_lines(BufReader::new(stderr), tx));

    let mut output = String::new();
    let url = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            tokio::select! {
                status = child.wait() => {
                    let status = status.expect("wait");
                    return Err(format!("exited {status}: {output}"));
                }
                line = rx.recv() => {
                    match line {
                        Some(line) => {
                            output.push_str(&line);
                            output.push('\n');
                            if let Some(url) = ready_url(&output) {
                                return Ok(url);
                            }
                        }
                        None => return Err(format!("pipes closed: {output}")),
                    }
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| Err(format!("not ready: {output}")))
    .unwrap_or_else(|message| panic!("{message}"));

    let response = reqwest::Client::new()
        .post(format!("{url}/api/host.describe"))
        .header("content-type", "application/json")
        .body(r#"{"type":"client-request","rpcId":"r1","method":"host.describe","payload":{}}"#)
        .send()
        .await
        .expect("host.describe");
    assert_eq!(response.status(), 200, "{}", response.status());
    let body = response.text().await.expect("body");
    assert!(
        body.contains("\"canOpenPath\":false"),
        "canOpenPath is not false: {body}"
    );

    child.kill().await.expect("kill");
    let _ = child.wait().await;
}
