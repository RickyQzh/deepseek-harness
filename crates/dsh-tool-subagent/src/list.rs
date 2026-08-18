//! Model-facing `list_agents` over continuable children.

use std::sync::Arc;

use dsh_agent::AgentRegistry;
use dsh_agent_loop::AgentStatus;
use dsh_session::{ContentBlock, SessionId};
use dsh_subagent::{ContinuableChildInfo, SubagentRuntime};
use dsh_tools::{ToolDefinition, ToolError, ToolExecution, ToolRuntime};
use serde_json::{Value, json};

const CONFIG_KEYS: &[&str] = &[];

/// Resolve YAML config. No keys are accepted.
///
/// # Errors
///
/// Unknown keys or a non-object.
pub fn resolve_config(value: &Value) -> Result<(), String> {
    crate::util::reject_unknown_keys(value, CONFIG_KEYS, "ToolSubagentListConfig")
}

/// Register `list_agents` on the shared parent tool runtime.
pub fn register_list_agents(
    tools: &mut ToolRuntime,
    subagents: Arc<SubagentRuntime>,
    agents: Arc<AgentRegistry>,
) {
    tools.register(list_agents_definition(subagents, agents));
}

fn list_agents_definition(
    subagents: Arc<SubagentRuntime>,
    agents: Arc<AgentRegistry>,
) -> ToolDefinition {
    ToolDefinition {
        name: "list_agents".into(),
        description:
            "List your continuable background subagents by durable id and label. Use it to recall which ones \
             you started, not to poll for completion — you are told when one finishes. Status comes from the live \
             registry: running means the agent is working right now, idle means it is loaded but between turns \
             (it may be waiting on agents it started), and ready means it exists only in storage — resumable, not \
             terminal, and not a result waiting to be collected; a `send_message` starts a new turn on the same \
             conversation, and a direct child remains a `send_message` candidate in every status. The snapshot is not a delivery \
             promise — `send_message` performs the authoritative check and may still fail. Children that could \
             not be read are reported as diagnostics instead of being silently dropped. Scope `descendants` \
             walks the whole tree below you in stable pre-order, annotating each entry with its durable direct-parent \
             session id and depth. You may use `send_message` only for depth-1 entries; deeper entries are \
             candidates for `interrupt_agent` only."
                .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "enum": ["children", "descendants"],
                    "description": "children (default) lists direct children only; descendants walks the complete tree below you."
                }
            }
        }),
        execute: Box::new(move |args, exec| {
            let subagents = Arc::clone(&subagents);
            let agents = Arc::clone(&agents);
            Box::pin(async move { execute_list_agents(&subagents, &agents, args, exec).await })
        }),
        render: Box::new(|args, value| {
            let scope = args
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("children");
            let entries = value.as_array();
            let text = match entries {
                Some(items) if items.is_empty() => "(no subagents)".to_string(),
                Some(items) => items
                    .iter()
                    .map(|entry| format_entry(scope, entry))
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => "(no subagents)".to_string(),
            };
            vec![ContentBlock::Text { text }]
        }),
        is_concurrency_safe: None,
    }
}

fn format_entry(scope: &str, entry: &Value) -> String {
    let id = entry.get("id").and_then(Value::as_str).unwrap_or("");
    let at = if scope == "descendants" {
        let parent = entry.get("parent").and_then(Value::as_str).unwrap_or("");
        let depth = entry.get("depth").and_then(Value::as_u64).unwrap_or(0);
        format!(" parent={parent} depth={depth}")
    } else {
        String::new()
    };
    match entry.get("kind").and_then(Value::as_str) {
        Some("child") => {
            let status = entry.get("status").and_then(Value::as_str).unwrap_or("");
            let label = entry.get("label").and_then(Value::as_str).unwrap_or("");
            format!("{id} [{status}]{at} — {label}")
        }
        _ => {
            let reason = entry.get("reason").and_then(Value::as_str).unwrap_or("");
            format!("{id} [diagnostic: {reason}]{at}")
        }
    }
}

async fn execute_list_agents(
    subagents: &SubagentRuntime,
    agents: &AgentRegistry,
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let Some(parent_id) = exec.session_id.as_ref() else {
        return Err(ToolError::Other(
            "list_agents requires a calling agent (exec.session_id was None)".into(),
        ));
    };
    let scope = args
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("children");
    let rows = match scope {
        "descendants" => list_descendants(subagents, parent_id.as_str()),
        _ => subagents
            .list_continuable(parent_id)
            .into_iter()
            .map(|row| (row, None))
            .collect(),
    };
    let mut entries = Vec::new();
    for (row, position) in rows {
        let status = match agents.status(row.child.as_str()).await {
            Ok(AgentStatus::Running) => "running",
            Ok(_) => "idle",
            Err(_) => "ready",
        };
        let mut entry = json!({
            "kind": "child",
            "id": row.child.as_str(),
            "label": row.label,
            "status": status,
        });
        if let Some((parent, depth)) = position {
            entry
                .as_object_mut()
                .expect("object")
                .insert("parent".into(), json!(parent));
            entry
                .as_object_mut()
                .expect("object")
                .insert("depth".into(), json!(depth));
        }
        entries.push(entry);
    }
    Ok(Value::Array(entries))
}

fn list_descendants(
    subagents: &SubagentRuntime,
    root: &str,
) -> Vec<(ContinuableChildInfo, Option<(String, u64)>)> {
    let mut out = Vec::new();
    walk_descendants(subagents, root, 1, &mut out);
    out
}

fn walk_descendants(
    subagents: &SubagentRuntime,
    parent: &str,
    depth: u64,
    out: &mut Vec<(ContinuableChildInfo, Option<(String, u64)>)>,
) {
    for row in subagents.list_continuable(&SessionId::new(parent)) {
        let child = row.child.as_str().to_string();
        out.push((row, Some((parent.to_string(), depth))));
        walk_descendants(subagents, &child, depth + 1, out);
    }
}
