//! Default one-shot summarization and durable checkpoint framing.

use dsh_llm::{BlockAssembler, GenerateOptions, LlmPurpose, LlmRuntime, ToolSchema};
use dsh_session::{ContentBlock, FinishReason, Message, MessageId, MessageRole, MessageSource};
use dsh_tools::AbortFlag;
use futures::StreamExt;
use std::sync::{Arc, Mutex};

use crate::config::BasicCompactionConfig;

/// Replayed conversation prefix the summarizer condenses.
#[derive(Clone, Debug)]
pub struct SummarizationInput {
    /// Conversation system prompt reused for prefix-cache alignment.
    pub system: Option<String>,
    /// Conversation tool schemas reused for prefix-cache alignment.
    pub tools: Option<Vec<ToolSchema>>,
    /// Shadowed region messages in surface order.
    pub messages: Vec<Message>,
    /// Routed provider from the latest request header, when present.
    pub provider: Option<String>,
    /// Routed model from the latest request header, when present.
    pub model: Option<String>,
}

/// Framing that makes the replacement user message established context.
pub const CHECKPOINT_PREAMBLE: &str = "This is an automatically generated checkpoint condensing an earlier span of the conversation to free up context. Treat the captured context as established background and build on it without restating it. Continue the task directly from the messages that follow, without acknowledging this checkpoint.";

/// Opening tag wrapping the structured summary inside the landed checkpoint node.
pub const SUMMARY_OPEN_TAG: &str = "<compacted-summary>";
/// Closing tag wrapping the structured summary inside the landed checkpoint node.
pub const SUMMARY_CLOSE_TAG: &str = "</compacted-summary>";

const COMPACTION_INSTRUCTION: &str = "You are now acting as a compaction engine for this AI coding assistant. Condense the conversation ABOVE into a structured checkpoint that lets another model resume the work with no loss of essential context.\n\
\n\
Output EXACTLY the Markdown structure below: keep every section, in order. Use terse bullets, not prose paragraphs. Write \"(none)\" for an empty section — never drop a section.\n\
\n\
## Primary Request and Intent\n\
- [the user's original and evolving goals; quote verbatim where the exact wording matters]\n\
\n\
## Key Technical Concepts\n\
- [technologies, frameworks, patterns, and conventions in play]\n\
\n\
## Files and Code\n\
- [exact path: why it matters, key changes or snippets]\n\
\n\
## Errors and Fixes\n\
- [error: how it was resolved, plus any related user feedback]\n\
\n\
## Pending Jobs\n\
- [explicitly requested work not yet completed]\n\
\n\
## Current Work\n\
- [precisely what was in progress at this checkpoint]\n\
\n\
## Next Step\n\
- [the single next action, directly in line with the most recent request, or \"(none)\"]\n\
\n\
## Critical Context\n\
- [decisions and their rationale, constraints, user preferences, open questions, data needed to continue]\n\
\n\
Rules:\n\
- Write concise English engineering prose. Preserve exact file paths, commands, error strings, identifiers, numeric values, function signatures, and syntax fragments.\n\
- Capture user feedback and explicit instructions faithfully, especially corrections.\n\
- Do NOT mention this summarization request or that the context was compacted.\n\
- Output only the checkpoint text: do not call any tool or take any other action.\n\
- If the conversation already contains a <compacted-summary> block, it is a PRIOR checkpoint. Do not copy it forward verbatim: preserve still-true facts, drop stale ones, and merge newer information into a single consolidated summary under the same structure.";

/// Safe summary plus the auxiliary call envelope recorded on `compaction/summary`.
#[derive(Clone, Debug)]
pub struct SummaryResult {
    /// Text-only projection stored on the checkpoint and summary event.
    pub summary: Vec<ContentBlock>,
    /// Complete provider output before the text-only projection.
    pub raw_output: Vec<ContentBlock>,
    /// Provider route used for the auxiliary call.
    pub provider: String,
    /// Model id used for the auxiliary call.
    pub model: String,
    /// Generation cap forwarded to the adapter.
    pub max_tokens: u64,
    /// Provider usage when the stream reported it.
    pub usage: Option<dsh_session::TokenUsage>,
}

/// Wrap raw summary blocks in the durable checkpoint framing.
#[must_use]
pub fn frame_summary(summary: &[ContentBlock]) -> Vec<ContentBlock> {
    let mut blocks = Vec::with_capacity(summary.len() + 2);
    blocks.push(ContentBlock::Text {
        text: format!("{CHECKPOINT_PREAMBLE}\n\n{SUMMARY_OPEN_TAG}"),
    });
    blocks.extend(summary.iter().cloned());
    blocks.push(ContentBlock::Text {
        text: SUMMARY_CLOSE_TAG.to_string(),
    });
    blocks
}

/// Run one cache-reusing `LlmRuntime::stream` summarization call.
pub async fn summarize_with_llm(
    llm: &Arc<Mutex<LlmRuntime>>,
    config: &BasicCompactionConfig,
    input: &SummarizationInput,
    session_id: dsh_session::SessionId,
    fallback_provider: &str,
    fallback_model: &str,
    signal: &AbortFlag,
) -> Result<SummaryResult, String> {
    let configured = if config.summarization_provider.is_empty() {
        None
    } else {
        Some((
            config.summarization_provider.as_str(),
            config.summarization_model.as_str(),
        ))
    };
    let header_target = input.provider.as_deref().zip(input.model.as_deref());
    let fallback = if fallback_provider.is_empty() || fallback_model.is_empty() {
        None
    } else {
        Some((fallback_provider, fallback_model))
    };
    let (provider, model) = match (configured, header_target, fallback) {
        (Some(target), _, _) | (None, Some(target), _) | (None, None, Some(target)) => target,
        (None, None, None) => {
            return Err(
                "no provider/model available for summarization: set both BasicCompactionConfig summarization fields, route one request, or set both AgentOptions fields"
                    .into(),
            );
        }
    };

    let mut messages = input.messages.clone();
    messages.push(Message {
        id: MessageId::new("compaction-instruction"),
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: COMPACTION_INSTRUCTION.to_string(),
        }],
        source: MessageSource::Plugin {
            plugin: "dsh-compaction-basic".into(),
            form: None,
            sections: Vec::new(),
            summary: None,
            compaction_id: None,
            source_command_id: None,
        },
    });
    let options = GenerateOptions {
        provider: provider.to_string(),
        model: model.to_string(),
        reasoning_effort: None,
        messages,
        system: input.system.clone(),
        tools: input.tools.clone(),
        temperature: None,
        max_tokens: Some(config.max_tokens),
        stop: None,
        signal: signal.clone(),
        session_id: Some(session_id),
        purpose: Some(LlmPurpose::Compaction),
    };
    let runtime = llm.lock().expect("llm").clone();
    let mut stream = runtime.stream(options);
    let mut assembler = BlockAssembler::new();
    while let Some(chunk) = stream.next().await {
        assembler.push(chunk);
    }
    if let Some(error) = finish_error(assembler.finish()) {
        return Err(error);
    }
    let raw_output = assembler.blocks();
    if content_has_image(&raw_output) {
        return Err("compaction summary cannot contain image output".into());
    }
    let summary: Vec<ContentBlock> = raw_output
        .iter()
        .filter(|block| matches!(block, ContentBlock::Text { .. }))
        .cloned()
        .collect();
    if !summary.iter().any(|block| match block {
        ContentBlock::Text { text } => !text.trim().is_empty(),
        _ => false,
    }) {
        return Err("summarization produced no text summary content".into());
    }
    Ok(SummaryResult {
        summary,
        raw_output,
        provider: provider.to_string(),
        model: model.to_string(),
        max_tokens: config.max_tokens,
        usage: assembler.usage().cloned(),
    })
}

fn finish_error(finish: FinishReason) -> Option<String> {
    match finish {
        FinishReason::Error { failure } | FinishReason::Aborted { failure } => {
            Some(failure.message)
        }
        FinishReason::MaxTokens => {
            Some("summarization truncated at the token cap (incomplete checkpoint)".into())
        }
        FinishReason::Stop | FinishReason::ToolCalls => None,
    }
}

fn content_has_image(blocks: &[ContentBlock]) -> bool {
    blocks.iter().any(|block| match block {
        ContentBlock::Image { .. } => true,
        ContentBlock::ToolResult { content, .. } => content_has_image(content),
        _ => false,
    })
}

/// Parse header `tools` JSON into [`ToolSchema`] values when it is a schema array.
#[must_use]
pub fn tools_from_header(value: Option<&serde_json::Value>) -> Option<Vec<ToolSchema>> {
    let value = value?;
    let items = value.as_array()?;
    let mut tools = Vec::with_capacity(items.len());
    for item in items {
        let name = item.get("name")?.as_str()?.to_string();
        let description = item
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let parameters = item
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        tools.push(ToolSchema {
            name,
            description,
            parameters,
        });
    }
    Some(tools)
}
