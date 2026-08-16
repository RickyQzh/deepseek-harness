//! Model-facing `read`, `write`, and `edit` filesystem tools for the DeepSeek Harness Rust host.

mod args;
mod edit;
mod error;
mod read;
mod write;

use std::sync::Arc;

use dsh_fs::{LocalFileSystem, ObservationGate, ObservationOwner};
use dsh_sandbox::SandboxExecutionPolicy;
use dsh_subprocess::LocalSubprocessRuntime;
use dsh_tools::ToolRuntime;

pub use error::remediate_fs_error;

/// Shared state cloned into each filesystem tool body.
#[derive(Clone)]
pub struct FsToolContext {
    /// Local UTF-8 backend used by read/write/edit.
    pub fs: Arc<LocalFileSystem>,
    /// Per-owner observation map that supplies write/edit intents.
    pub gate: Arc<ObservationGate>,
    /// Observation owner recorded on successful reads and mutations.
    pub owner: ObservationOwner,
    /// Standing sandbox policy stamped onto mutations; `None` when unfenced.
    pub sandbox: Option<SandboxExecutionPolicy>,
    /// Subprocess runtime retained for `search` (not registered here).
    pub subprocess: Arc<LocalSubprocessRuntime>,
    /// `rg` executable name or path retained for `search`.
    pub rg_binary: String,
}

/// Register model-facing `read`, `write`, and `edit` on `runtime`.
///
/// Does not register `search`. Each body clones `ctx`. Read may overlap other
/// parallel calls; write and edit are exclusive.
pub fn register_fs_tools(runtime: &mut ToolRuntime, ctx: FsToolContext) {
    runtime.register(read::definition(ctx.clone()));
    runtime.register(write::definition(ctx.clone()));
    runtime.register(edit::definition(ctx));
}

#[cfg(test)]
mod tests {
    use super::{FsToolContext, register_fs_tools};
    use dsh_fs::{LocalFileSystem, ObservationGate, ObservationOwner};
    use dsh_session::{CallId, ContentBlock};
    use dsh_tools::{AbortFlag, ToolExecutionInput, ToolPresentationMode, ToolRuntime};
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    fn runtime_and_ctx() -> (ToolRuntime, FsToolContext, std::path::PathBuf) {
        let dir = {
            let d = std::env::temp_dir().join(format!(
                "dsh-tool-fs-{}-{}",
                std::process::id(),
                TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&d).unwrap();
            d
        };
        let ctx = FsToolContext {
            fs: Arc::new(LocalFileSystem::new(&dir)),
            gate: Arc::new(ObservationGate::new()),
            owner: ObservationOwner(1),
            sandbox: None,
            subprocess: Arc::new(dsh_subprocess::LocalSubprocessRuntime::new()),
            rg_binary: "rg".into(),
        };
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        register_fs_tools(&mut tools, ctx.clone());
        (tools, ctx, dir)
    }

    fn input(name: &str, args: serde_json::Value) -> ToolExecutionInput {
        ToolExecutionInput {
            call_id: CallId::new("c1"),
            root_call_id: None,
            name: name.into(),
            arguments: args,
            parent: None,
            signal: AbortFlag::new(),
        }
    }

    #[tokio::test]
    async fn edit_without_read_is_fs_not_observed() {
        let (mut tools, _ctx, dir) = runtime_and_ctx();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let result = tools
            .execute(input(
                "edit",
                json!({
                    "file_path": "a.txt",
                    "old_string": "hello",
                    "new_string": "world"
                }),
            ))
            .await;
        assert!(result.is_error());
        match result {
            dsh_tools::ToolExecutionResult::Failure { error, .. } => {
                assert_eq!(error.info.as_ref().unwrap().code, "FS_NOT_OBSERVED");
                assert!(error.message.contains("read the file, then retry"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn read_then_edit_succeeds() {
        let (mut tools, _ctx, dir) = runtime_and_ctx();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let read = tools
            .execute(input("read", json!({"file_path": "a.txt"})))
            .await;
        assert!(!read.is_error());
        let edited = tools
            .execute(input(
                "edit",
                json!({
                    "file_path": "a.txt",
                    "old_string": "hello",
                    "new_string": "world"
                }),
            ))
            .await;
        assert!(!edited.is_error());
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "world");
    }

    #[tokio::test]
    async fn read_missing_is_fs_not_found() {
        let (mut tools, _ctx, _dir) = runtime_and_ctx();
        let result = tools
            .execute(input("read", json!({"file_path": "missing.txt"})))
            .await;
        match result {
            dsh_tools::ToolExecutionResult::Failure { error, .. } => {
                assert_eq!(error.info.as_ref().unwrap().code, "FS_NOT_FOUND");
                assert!(error.message.contains("not found"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn read_renders_numbered_lines() {
        let (mut tools, _ctx, dir) = runtime_and_ctx();
        std::fs::write(dir.join("a.txt"), "hello\nworld\n").unwrap();
        let result = tools
            .execute(input("read", json!({"file_path": "a.txt"})))
            .await;
        assert!(!result.is_error());
        let ContentBlock::Text { text } = &result.content()[0] else {
            panic!("expected text");
        };
        assert!(text.contains("     1|hello"));
        assert!(text.contains("     2|world"));
        assert!(text.contains("     3|"));
    }

    #[tokio::test]
    async fn write_creates_without_prior_read() {
        let (mut tools, _ctx, dir) = runtime_and_ctx();
        let result = tools
            .execute(input(
                "write",
                json!({"file_path": "new.txt", "content": "hi"}),
            ))
            .await;
        assert!(!result.is_error());
        assert_eq!(std::fs::read_to_string(dir.join("new.txt")).unwrap(), "hi");
        let ContentBlock::Text { text } = &result.content()[0] else {
            panic!("expected text");
        };
        assert!(text.contains("Created file"));
    }

    #[tokio::test]
    async fn write_existing_without_read_is_fs_not_observed() {
        let (mut tools, _ctx, dir) = runtime_and_ctx();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let result = tools
            .execute(input(
                "write",
                json!({"file_path": "a.txt", "content": "world"}),
            ))
            .await;
        match result {
            dsh_tools::ToolExecutionResult::Failure { error, .. } => {
                assert_eq!(error.info.as_ref().unwrap().code, "FS_NOT_OBSERVED");
                assert!(error.message.contains("read the file, then retry"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn unconfined_escalation_is_invalid() {
        let (mut tools, _ctx, _dir) = runtime_and_ctx();
        let result = tools
            .execute(input(
                "write",
                json!({
                    "file_path": "a.txt",
                    "content": "x",
                    "sandbox_permissions": "danger-full-access",
                    "justification": "need it"
                }),
            ))
            .await;
        match result {
            dsh_tools::ToolExecutionResult::Failure { error, .. } => {
                assert!(error.message.contains(
                    "sandbox_permissions is not available in this composition (no sandboxing filesystem to escalate)"
                ));
            }
            other => panic!("{other:?}"),
        }
    }
}
