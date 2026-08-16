//! Model-facing `skill` tool and durable session catalog.

use std::sync::{Arc, Mutex};

use dsh_agent_loop::{CompactionScope, EVENT_AGENT_PRE_STEP, PreStepDecision};
use dsh_kernel::{Context, KernelError};
use dsh_session::{
    ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, Session, SessionEvent,
    SkillCatalogEntry,
};
use dsh_tools::{ToolDefinition, ToolError, ToolRuntime};
use serde_json::{Value, json};

use crate::{
    SkillError, SkillRegistry, is_skill_name, registry::escape_text, render_skill_content,
};

/// Default maximum normalized description length rendered in the session catalog.
pub const DEFAULT_CATALOG_DESCRIPTION_MAX_LENGTH: usize = 500;

const SKILL_TOOL_NAME: &str = "skill";

/// Validated catalog description cap. Minimum 3.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogConfig {
    /// Maximum normalized description length rendered in the session catalog.
    pub description_max_length: usize,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            description_max_length: DEFAULT_CATALOG_DESCRIPTION_MAX_LENGTH,
        }
    }
}

/// Register the `skill` tool on `runtime`.
pub fn register_skill_tool(runtime: &mut ToolRuntime, skills: Arc<SkillRegistry>) {
    runtime.register(skill_definition(skills));
}

/// Register the first-pre-step catalog listener. Call `next` after prepend.
///
/// # Errors
///
/// [`KernelError::InactiveEffect`] when this fiber cannot register listeners.
pub fn register_catalog_listener(
    ctx: &Context,
    skills: Arc<SkillRegistry>,
    tools: Arc<Mutex<ToolRuntime>>,
    config: CatalogConfig,
) -> Result<(), KernelError> {
    ctx.on_waterfall::<PreStepDecision, _, _>(EVENT_AGENT_PRE_STEP, move |decision, next| {
        let skills = Arc::clone(&skills);
        let tools = Arc::clone(&tools);
        async move {
            let decision = prepend_catalog(decision, &skills, &tools, config);
            next(decision).await
        }
    })
    .map(|_| ())
}

fn skill_definition(skills: Arc<SkillRegistry>) -> ToolDefinition {
    ToolDefinition {
        name: SKILL_TOOL_NAME.into(),
        description: "Load the full instructions for an available skill. Call this with the exact skill name from the session skill catalog before acting on a task that names or clearly matches that skill.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The exact skill name from the available skills list."
                }
            },
            "required": ["name"]
        }),
        execute: Box::new(move |args, _exec| {
            let skills = Arc::clone(&skills);
            Box::pin(async move { execute_skill_tool(&skills, args) })
        }),
        render: Box::new(|_args, value| {
            let name = value.get("name").and_then(Value::as_str).unwrap_or("");
            let provider = value.get("provider").and_then(Value::as_str).unwrap_or("");
            let content = value.get("content").and_then(Value::as_str).unwrap_or("");
            vec![ContentBlock::Text {
                text: render_skill_content(name, provider, content),
            }]
        }),
        is_concurrency_safe: Some(Box::new(|_| true)),
    }
}

fn execute_skill_tool(skills: &SkillRegistry, args: Value) -> Result<Value, ToolError> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Other("skill: name is required".into()))?;
    if !is_skill_name(name) {
        return Err(ToolError::Other(format!("invalid skill name \"{name}\"")));
    }
    let cwd = current_cwd();
    let cwd_ref = cwd.as_deref();
    let summary = skills
        .list(cwd_ref)
        .into_iter()
        .find(|skill| skill.name == name);
    let Some(summary) = summary else {
        return Err(unknown_tool_error(name));
    };
    if !summary.invocation.model_invocable {
        return Err(ToolError::Other(format!(
            "skill \"{name}\" is not available for model invocation"
        )));
    }
    let skill = match skills.get(name, cwd_ref) {
        Ok(skill) => skill,
        Err(SkillError::Unknown(_)) => return Err(unknown_tool_error(name)),
        Err(error) => return Err(ToolError::Other(error.to_string())),
    };
    if !skill.summary.invocation.model_invocable {
        return Err(ToolError::Other(format!(
            "skill \"{name}\" is not available for model invocation"
        )));
    }
    Ok(json!({
        "name": skill.summary.name,
        "provider": skill.summary.provider,
        "content": skill.content,
    }))
}

fn unknown_tool_error(name: &str) -> ToolError {
    ToolError::Other(format!(
        "skill \"{name}\" is unknown or no longer available"
    ))
}

fn current_cwd() -> Option<String> {
    CompactionScope::try_current(|scope| scope.with_session(|session| session.header().cwd.clone()))
        .flatten()
}

fn prepend_catalog(
    decision: PreStepDecision,
    skills: &SkillRegistry,
    tools: &Mutex<ToolRuntime>,
    config: CatalogConfig,
) -> PreStepDecision {
    let PreStepDecision::Enter { mut messages } = decision else {
        return decision;
    };
    if messages.is_empty() {
        return PreStepDecision::Enter { messages };
    }
    let Some((cwd, published)) = CompactionScope::try_current(|scope| {
        scope.with_session(|session| (session.header().cwd.clone(), catalog_published(session)))
    }) else {
        return PreStepDecision::Enter { messages };
    };
    if published || !skill_tool_visible(tools) {
        return PreStepDecision::Enter { messages };
    }
    let entries: Vec<SkillCatalogEntry> = skills
        .list(cwd.as_deref())
        .into_iter()
        .filter(|skill| skill.invocation.model_invocable)
        .map(|skill| SkillCatalogEntry {
            name: skill.name,
            description: catalog_description(&skill.description, config.description_max_length),
        })
        .collect();
    if entries.is_empty() {
        return PreStepDecision::Enter { messages };
    }
    messages.insert(0, catalog_message(&entries));
    PreStepDecision::Enter { messages }
}

fn skill_tool_visible(tools: &Mutex<ToolRuntime>) -> bool {
    tools
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .registered_names()
        .iter()
        .any(|name| name == SKILL_TOOL_NAME)
}

fn catalog_published(session: &Session) -> bool {
    session.events().iter().any(|event| match event {
        LogEvent::Known(SessionEvent::UserMessage { data, .. }) => {
            matches!(data.source, MessageSource::SkillCatalog { .. })
        }
        _ => false,
    })
}

fn catalog_message(entries: &[SkillCatalogEntry]) -> Message {
    Message {
        id: mint_message_id(),
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: render_catalog_message(entries),
        }],
        source: MessageSource::SkillCatalog {
            form: "catalog".into(),
            update: None,
            entries: entries.to_vec(),
        },
    }
}

fn render_catalog_message(entries: &[SkillCatalogEntry]) -> String {
    let mut lines = vec![
        "<system-reminder>".to_string(),
        "A skill is a reusable set of task-specific instructions. The following skills are available in this session:".to_string(),
        String::new(),
        "<available_skills>".to_string(),
    ];
    lines.extend(render_catalog_entries(entries));
    lines.push("</available_skills>".to_string());
    lines.push(String::new());
    lines.push("If the user names a skill, or the task clearly matches a skill's description, call the `skill` tool with the exact skill name before taking task actions. Load all applicable skills, then follow their full instructions. This catalog contains summaries only; do not infer or follow a skill's instructions until it has been loaded.".to_string());
    lines.push("A user may also invoke a skill directly; its <skill_content> block then appears in this conversation. Follow it, and do not call the `skill` tool again for that skill.".to_string());
    lines.push("</system-reminder>".to_string());
    lines.join("\n")
}

fn render_catalog_entries(entries: &[SkillCatalogEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| format!("- `{}`: {}", entry.name, escape_text(&entry.description)))
        .collect()
}

fn catalog_description(value: &str, max_length: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_length {
        return normalized;
    }
    let take = max_length.saturating_sub(3);
    let mut truncated: String = normalized.chars().take(take).collect();
    truncated.push_str("...");
    truncated
}

fn mint_message_id() -> MessageId {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    MessageId::new(format!("skill-catalog-{}-{}", std::process::id(), nanos))
}

#[cfg(test)]
// `execute` borrows `&mut ToolRuntime` for the future; these test mutexes have no other waiters.
#[allow(clippy::await_holding_lock)]
mod tests {
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_session::CallId;
    use dsh_tools::{AbortFlag, ToolExecutionInput, ToolExecutionResult, ToolRuntime};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    use crate::plugin::{register_skill, register_tool_skill};

    async fn boot_skill_tools() -> Arc<Mutex<ToolRuntime>> {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_tools::plugin::register(&mut registry);
        register_skill(&mut registry);
        register_tool_skill(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-tools'\n- name: '@deepseek-ai/dsh-skill'\n- name: '@deepseek-ai/dsh-tool-skill'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot skill tools");
        ctx.inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools")
    }

    async fn execute_skill(tools: &Arc<Mutex<ToolRuntime>>, name: &str) -> ToolExecutionResult {
        tools
            .lock()
            .expect("tools")
            .execute(ToolExecutionInput {
                call_id: CallId::new("c1"),
                root_call_id: None,
                name: "skill".into(),
                arguments: json!({ "name": name }),
                parent: None,
                session_id: None,
                signal: AbortFlag::new(),
            })
            .await
    }

    #[tokio::test]
    async fn skill_tool_returns_unknown_for_missing_name() {
        let tools = boot_skill_tools().await;
        let result = execute_skill(&tools, "no-such-skill").await;
        assert!(result.is_error());
    }
}
