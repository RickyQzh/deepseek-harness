//! Child-scoped `report` tool, installed only on continuable child tool runtimes.

use std::sync::Arc;

use dsh_agent::AgentRegistry;
use dsh_session::{ContentBlock, Message, MessageRole, MessageSource};
use dsh_subagent::SubagentRuntime;
use dsh_tools::{ToolDefinition, ToolError, ToolExecution, ToolRuntime};
use serde_json::{Value, json};

use crate::util::mint_message_id;

/// Parent scheduling for an accepted report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportDelivery {
    /// Open a follow-up turn on the parent.
    Wakeup,
    /// Inject without latching a wake.
    Quiet,
}

/// Validated report-tool configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReportConfig {
    /// How accepted reports are scheduled on the parent.
    pub report_delivery: ReportDelivery,
}

const CONFIG_KEYS: &[&str] = &["reportDelivery"];

/// Resolve YAML config. Default delivery is `wakeup`.
///
/// # Errors
///
/// Unknown keys or invalid `reportDelivery`.
pub fn resolve_config(value: &Value) -> Result<ReportConfig, String> {
    crate::util::reject_unknown_keys(value, CONFIG_KEYS, "ToolSubagentReportConfig")?;
    match value {
        Value::Null => Ok(ReportConfig {
            report_delivery: ReportDelivery::Wakeup,
        }),
        Value::Object(map) => {
            let report_delivery =
                match map.get("reportDelivery") {
                    None | Some(Value::Null) => ReportDelivery::Wakeup,
                    Some(Value::String(text)) if text == "wakeup" => ReportDelivery::Wakeup,
                    Some(Value::String(text)) if text == "quiet" => ReportDelivery::Quiet,
                    Some(_) => return Err(
                        "ToolSubagentReportConfig.reportDelivery must be \"wakeup\" or \"quiet\""
                            .into(),
                    ),
                };
            Ok(ReportConfig { report_delivery })
        }
        _ => Err("ToolSubagentReportConfig: config must be an object".into()),
    }
}

/// Install `report` onto each continuable child's cloned tool runtime.
pub fn register_report_setup(
    subagents: &SubagentRuntime,
    agents: Arc<AgentRegistry>,
    config: ReportConfig,
) {
    subagents.register_continuable_setup(Arc::new(move |tools: &mut ToolRuntime| {
        tools.register(report_definition(Arc::clone(&agents), config));
    }));
}

fn report_definition(agents: Arc<AgentRegistry>, config: ReportConfig) -> ToolDefinition {
    ToolDefinition {
        name: "report".into(),
        description:
            "Report selected content to the agent that started you. Call this once before you finish, with a \
             self-contained final result, and earlier for progress or findings that change what that agent does \
             next. That agent shares your workspace but does not automatically receive your transcript, tool \
             output, or reasoning, so finishing your work is not itself a result. Reporting does not end your \
             turn or finish your work, and only your direct parent receives it. A failed call may still have \
             arrived, so do not blindly repeat it."
                .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "output": {
                    "type": "string",
                    "description": "Actionable content for your parent; summarize conclusions and reference relevant shared paths."
                }
            },
            "required": ["output"]
        }),
        execute: Box::new(move |args, exec| {
            let agents = Arc::clone(&agents);
            Box::pin(async move { execute_report(&agents, config, args, exec).await })
        }),
        render: Box::new(|_args, value| {
            let id = value
                .get("messageId")
                .and_then(Value::as_str)
                .unwrap_or("");
            vec![ContentBlock::Text {
                text: format!("report accepted by the agent that started you as message {id}"),
            }]
        }),
        is_concurrency_safe: None,
    }
}

async fn execute_report(
    agents: &AgentRegistry,
    config: ReportConfig,
    args: Value,
    exec: ToolExecution,
) -> Result<Value, ToolError> {
    let output = match args.get("output").and_then(Value::as_str) {
        Some(text) => text.to_string(),
        None => return Err(ToolError::Other("output is required".into())),
    };
    let Some(child_id) = exec.session_id.as_ref() else {
        return Err(ToolError::Other(
            "report requires a calling agent (exec.session_id was None)".into(),
        ));
    };
    let child = agents
        .get(child_id.as_str())
        .ok_or_else(|| ToolError::Other("report requires a live calling child agent".into()))?;
    let parent_id = {
        let agent = child.lock();
        agent.session.header().parent_session.clone()
    };
    let Some(parent_id) = parent_id else {
        return Err(ToolError::Other(
            "direct parent is not live; report was not delivered".into(),
        ));
    };
    let parent = agents.get(parent_id.as_str()).ok_or_else(|| {
        ToolError::Other("direct parent is not live; report was not delivered".into())
    })?;
    let message_id = mint_message_id("report");
    let framed = format!(
        "Background subagent {} reported:\n{output}",
        child_id.as_str()
    );
    let message = Message {
        id: message_id.clone(),
        role: MessageRole::User,
        content: vec![ContentBlock::Text { text: framed }],
        source: MessageSource::SubagentReport {
            form: "relay".into(),
            sender_session_id: child_id.as_str().to_string(),
        },
    };
    match config.report_delivery {
        ReportDelivery::Wakeup => parent
            .followup(message)
            .await
            .map_err(|error| ToolError::Other(error.to_string()))?,
        ReportDelivery::Quiet => parent
            .inject(message)
            .await
            .map_err(|error| ToolError::Other(error.to_string()))?,
    }
    Ok(json!({ "messageId": message_id.as_str() }))
}
