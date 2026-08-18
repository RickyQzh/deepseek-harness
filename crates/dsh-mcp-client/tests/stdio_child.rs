//! Spawn the crate-local MCP fixture over stdio.

use dsh_mcp_client::spawn_stdio;
use serde_json::{Value, json};

#[tokio::test]
async fn stdio_child_add_round_trip() {
    let _lock = dsh_mcp_client::lock_stdio_fixture_tests().await;
    let program = env!("CARGO_BIN_EXE_dsh-mcp-fixture");
    let (mut session, mut child) = spawn_stdio(program, &[], &[], None).expect("spawn fixture");
    session.initialize().await.expect("initialize");
    let result = session
        .call_tool("add", json!({"a": 2, "b": 3}))
        .await
        .expect("call_tool");
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| blocks.first())
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str);
    assert_eq!(text, Some("5"));
    drop(session);
    child.start_kill().expect("kill");
    let _ = child.wait().await.expect("wait");
}
