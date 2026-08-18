//! MCP `tools/list` generation swap onto [`dsh_tools::ToolRuntime`].

use std::collections::HashSet;
use std::time::Duration;

use dsh_session::ContentBlock;
use dsh_tools::{RegisterError, ToolDefinition, ToolError, ToolRuntime};
use serde_json::{Value, json};

use crate::client::{McpSession, McpToolDraft};
use crate::name::public_tool_name;
use crate::result::extract_text;
use crate::rpc::McpRpcError;

/// Failure swapping an MCP tool generation into [`ToolRuntime`].
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// `tools/list` repeated a raw name or two raw names share one public name.
    #[error("{0}")]
    InvalidToolList(String),
    /// A new public name is already registered and is not in `previous`.
    #[error("{0}")]
    ForeignName(String),
    /// `try_register` hit a duplicate after `previous` was unregistered.
    #[error("{0}")]
    Duplicate(String),
    /// MCP JSON-RPC `tools/list` failed.
    #[error(transparent)]
    Rpc(#[from] McpRpcError),
}

/// Fetch `tools/list` and swap that generation onto `tools`.
///
/// Leaves `previous` registered when the list is invalid or a new public name
/// is already owned by a foreign registration. Otherwise unregisters `previous`,
/// then `try_register`s each new definition. A duplicate during that attempt
/// unregisters names registered in this call and fails.
///
/// # Parameters
///
/// * `session` - Connected MCP session used for `tools/list` and later `tools/call`.
/// * `tools` - Runtime that receives public `mcp__` names.
/// * `server_name` - Local YAML `serverName` used in public names and diagnostics.
/// * `previous` - Public names from the prior successful generation.
///
/// # Returns
///
/// Live public names after a successful swap.
///
/// # Errors
///
/// [`SyncError`] when the list is invalid, a foreign name squats, registration
/// duplicates, or `tools/list` fails.
pub async fn sync_tools(
    session: &mut McpSession,
    tools: &mut ToolRuntime,
    server_name: &str,
    previous: Vec<String>,
) -> Result<Vec<String>, SyncError> {
    let drafts = session.list_tools().await?;
    swap_generation(session, tools, server_name, previous, drafts)
}

/// Apply a fetched `tools/list` onto `tools` without awaiting.
pub(crate) fn swap_generation(
    session: &McpSession,
    tools: &mut ToolRuntime,
    server_name: &str,
    previous: Vec<String>,
    drafts: Vec<McpToolDraft>,
) -> Result<Vec<String>, SyncError> {
    let timeout = session.tool_call_timeout();
    let mut pending: Vec<(String, ToolDefinition)> = Vec::new();
    let mut seen_raw = HashSet::new();
    let mut seen_public = HashSet::new();
    for draft in drafts {
        let raw = draft.name().to_string();
        let public = public_tool_name(server_name, &raw);
        if !seen_raw.insert(raw.clone()) || !seen_public.insert(public.clone()) {
            return Err(SyncError::InvalidToolList(format!(
                "mcp-client({server_name}): server listed tool \"{raw}\" more than once — invalid tool list"
            )));
        }
        pending.push((
            public.clone(),
            mcp_tool_definition(session.clone(), draft, public, timeout),
        ));
    }

    let previous_set: HashSet<&str> = previous.iter().map(String::as_str).collect();
    let live = tools.registered_names();
    for (public, _) in &pending {
        let foreign = live.iter().any(|name| name == public);
        if foreign && !previous_set.contains(public.as_str()) {
            return Err(SyncError::ForeignName(format!(
                "mcp-client({server_name}): public tool name \"{public}\" is already registered"
            )));
        }
    }

    for name in &previous {
        tools.unregister(name);
    }

    let mut registered = Vec::new();
    for (public, definition) in pending {
        match tools.try_register(definition) {
            Ok(()) => registered.push(public),
            Err(RegisterError::Duplicate(name)) => {
                for owned in &registered {
                    tools.unregister(owned);
                }
                return Err(SyncError::Duplicate(format!(
                    "mcp-client({server_name}): tool \"{name}\" is already registered"
                )));
            }
        }
    }
    Ok(registered)
}

fn mcp_tool_definition(
    session: McpSession,
    draft: McpToolDraft,
    public: String,
    timeout: Duration,
) -> ToolDefinition {
    let raw = draft.name().to_string();
    let task_required = draft.task_required();
    let description = draft.description().to_string();
    let parameters = draft.input_schema().clone();
    let execute_raw = raw.clone();
    let execute_public = public.clone();
    let render_public = public.clone();
    ToolDefinition {
        name: public,
        description,
        parameters,
        execute: Box::new(move |args, _exec| {
            let mut session = session.clone();
            let raw = execute_raw.clone();
            let public = execute_public.clone();
            Box::pin(async move {
                execute_mcp_tool(&mut session, &raw, &public, task_required, args, timeout).await
            })
        }),
        render: Box::new(move |_args, value| {
            let text = match value.get("content").and_then(Value::as_array) {
                Some(blocks) => extract_text(blocks, &render_public),
                None => extract_text(&[], &render_public),
            };
            vec![ContentBlock::Text { text }]
        }),
        is_concurrency_safe: None,
    }
}

async fn execute_mcp_tool(
    session: &mut McpSession,
    raw: &str,
    public: &str,
    task_required: bool,
    args: Value,
    timeout: Duration,
) -> Result<Value, ToolError> {
    if task_required {
        return Err(ToolError::Other(format!(
            "Tool \"{raw}\" requires task-based execution, which this bridge does not support"
        )));
    }
    let arguments = if args.is_object() { args } else { json!({}) };
    let result = match tokio::time::timeout(timeout, session.call_tool(raw, arguments)).await {
        Ok(Ok(value)) => value,
        Ok(Err(err)) => return Err(ToolError::Other(err.to_string())),
        Err(_) => {
            return Err(ToolError::Other(format!(
                "MCP tools/call timed out for \"{raw}\""
            )));
        }
    };
    let text = match result.get("content").and_then(Value::as_array) {
        Some(blocks) => extract_text(blocks, public),
        None => extract_text(&[], public),
    };
    match result.get("isError").and_then(Value::as_bool) {
        Some(true) => Err(ToolError::Other(text)),
        _ => Ok(result),
    }
}

#[cfg(test)]
mod tests {
    use super::sync_tools;
    use crate::public_tool_name;
    use crate::test_server::{spawn_loopback, spawn_loopback_with_list};
    use dsh_session::{CallId, ContentBlock};
    use dsh_tools::{
        AbortFlag, ToolDefinition, ToolExecutionInput, ToolPresentationMode, ToolRuntime,
    };
    use serde_json::{Value, json};

    fn dummy(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: name.into(),
            parameters: json!({}),
            execute: Box::new(|_args, _exec| Box::pin(async move { Ok(json!({})) })),
            render: Box::new(|_args, _value| vec![]),
            is_concurrency_safe: None,
        }
    }

    fn execute_input(name: &str, args: Value) -> ToolExecutionInput {
        ToolExecutionInput {
            call_id: CallId::new("c1"),
            root_call_id: None,
            name: name.into(),
            arguments: args,
            parent: None,
            session_id: None,
            signal: AbortFlag::new(),
        }
    }

    fn add_tool_draft() -> Value {
        json!({
            "name": "add",
            "description": "Adds two numbers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "a": { "type": "number" },
                    "b": { "type": "number" }
                },
                "required": ["a", "b"]
            }
        })
    }

    #[tokio::test]
    async fn invalid_tool_list_keeps_previous_generation() {
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.try_register(dummy("keep-me")).expect("previous");
        let (mut session, _fixture) = spawn_loopback_with_list(json!({
            "tools": [add_tool_draft(), add_tool_draft()]
        }));
        session.initialize().await.expect("initialize");
        let err = sync_tools(
            &mut session,
            &mut tools,
            "NAME",
            vec!["keep-me".to_string()],
        )
        .await
        .expect_err("duplicate raw tool list");
        assert_eq!(
            err.to_string(),
            "mcp-client(NAME): server listed tool \"add\" more than once — invalid tool list"
        );
        assert!(
            tools
                .registered_names()
                .iter()
                .any(|name| name == "keep-me"),
            "previous generation must stay registered"
        );
    }

    #[tokio::test]
    async fn is_error_result_becomes_tool_error() {
        let (mut session, _fixture) = spawn_loopback();
        session.initialize().await.expect("initialize");
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        sync_tools(&mut session, &mut tools, "srv", Vec::new())
            .await
            .expect("sync");
        let public_fail = public_tool_name("srv", "fail");
        let result = tools.execute(execute_input(&public_fail, json!({}))).await;
        assert!(result.is_error(), "MCP isError must become a tool error");
        let expected = crate::extract_text(
            &[json!({"type": "text", "text": "Something went wrong"})],
            &public_fail,
        );
        assert_eq!(
            result.content(),
            &[ContentBlock::Text {
                text: format!("Error: {expected}"),
            }]
        );
    }
}
