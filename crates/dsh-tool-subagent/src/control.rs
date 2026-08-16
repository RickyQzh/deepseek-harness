//! Model-facing `send_message` over continuable children.

use std::sync::Arc;

use dsh_agent::AgentRegistry;
use dsh_session::{ContentBlock, Message, MessageRole, MessageSource, SessionId};
use dsh_subagent::SubagentRuntime;
use dsh_tools::{ToolDefinition, ToolError, ToolExecution, ToolRuntime};
use serde_json::{Value, json};

use crate::util::mint_message_id;

const CONFIG_KEYS: &[&str] = &[];

/// Resolve YAML config. No keys are accepted.
///
/// # Errors
///
/// Unknown keys or a non-object.
pub fn resolve_config(value: &Value) -> Result<(), String> {
    crate::util::reject_unknown_keys(value, CONFIG_KEYS, "ToolSubagentControlConfig")
}

/// Register `send_message` on the shared parent tool runtime.
pub fn register_send_message(
    tools: &mut ToolRuntime,
    subagents: Arc<SubagentRuntime>,
    agents: Arc<AgentRegistry>,
) {
    tools.register(send_message_definition(subagents, agents));
}

fn send_message_definition(
    subagents: Arc<SubagentRuntime>,
    agents: Arc<AgentRegistry>,
) -> ToolDefinition {
    ToolDefinition {
        name: "send_message".into(),
        description:
            "Send a message to a background subagent by its subagent id, continuing the same conversation. It \
             becomes the subagent's next turn: if it is still working, the message waits until its current turn \
             finishes, so it cannot redirect work already underway. This call returns no answer from the \
             subagent — only confirmation that the message was delivered — so use it to give it more work. A \
             failure means the message was NOT delivered."
                .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "subagent_id": {
                    "type": "string",
                    "description": "The subagent id returned when the background subagent was started."
                },
                "message": {
                    "type": "string",
                    "description": "The message to deliver to the subagent."
                }
            },
            "required": ["subagent_id", "message"]
        }),
        execute: Box::new(move |args, exec| {
            let subagents = Arc::clone(&subagents);
            let agents = Arc::clone(&agents);
            Box::pin(async move { execute_send_message(&subagents, &agents, args, exec).await })
        }),
        render: Box::new(|args, _value| {
            let id = args
                .get("subagent_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            vec![ContentBlock::Text {
                text: format!("message queued as the next turn for subagent {id}"),
            }]
        }),
        is_concurrency_safe: None,
    }
}

async fn execute_send_message(
    subagents: &SubagentRuntime,
    agents: &AgentRegistry,
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let subagent_id = match args.get("subagent_id").and_then(Value::as_str) {
        Some(text) if !text.is_empty() => SessionId::new(text),
        _ => return Err(ToolError::Other("subagent_id is required".into())),
    };
    let body = match args.get("message").and_then(Value::as_str) {
        Some(text) => text.to_string(),
        None => return Err(ToolError::Other("message is required".into())),
    };
    let Some(parent_id) = exec.session_id.as_ref() else {
        return Err(ToolError::Other(
            "send_message requires a calling agent (exec.session_id was None)".into(),
        ));
    };
    let parent = agents.get(parent_id.as_str()).ok_or_else(|| {
        ToolError::Other("send_message requires a calling agent (exec.agent was undefined)".into())
    })?;
    let message_id = mint_message_id("coord");
    let message = Message {
        id: message_id.clone(),
        role: MessageRole::User,
        content: vec![ContentBlock::Text { text: body }],
        source: MessageSource::Coordinator {
            form: "relay".into(),
            sender_session_id: parent_id.as_str().to_string(),
        },
    };
    subagents
        .followup_child(&subagent_id, message, &parent)
        .await
        .map_err(|error| ToolError::Other(error.to_string()))?;
    Ok(json!({ "messageId": message_id.as_str() }))
}
