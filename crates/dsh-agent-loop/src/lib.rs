//! Scripted agent loop: inbox, phase machine, reconstruction, and tool scheduling for the Rust host.

mod agent;
mod compaction_scope;
mod error;
mod inbox;
mod runtime_context;
mod tool_calls;

#[cfg(test)]
mod phase3_exit;

#[cfg(test)]
mod phase6_exit;

pub use agent::{
    AgentStatus, CancelCause, CancelOptions, DEFAULT_MAX_PARALLEL_TOOL_CALLS, EVENT_AGENT_PRE_STEP,
    EVENT_AGENT_REQUEST_ERROR, LoopAgent, LoopOptions, Phase, PreStepDecision, PreStepPayload,
    RequestErrorAction, RequestErrorPayload,
};
pub use compaction_scope::CompactionScope;
pub use error::LoopError;
pub use inbox::Inbox;
pub use runtime_context::{
    RUNTIME_CONTEXT_CLEARED, RUNTIME_CONTEXT_SOURCE, RuntimeContextProjection,
};

#[cfg(test)]
use dsh_session::{
    ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, SESSION_FORMAT_VERSION,
    SessionHeader, SessionId,
};
use dsh_session::{Session, SessionEvent};

/// Shared test session header: format version 0, `created_at = 1`.
#[cfg(test)]
#[must_use]
pub(crate) fn test_header(id: &str) -> SessionHeader {
    SessionHeader {
        version: SESSION_FORMAT_VERSION,
        id: SessionId::new(id),
        created_at: 1,
        cwd: None,
        parent_session: None,
        seed_length: None,
        origin: None,
        delegation_depth: None,
        agent_preset: None,
    }
}

/// Next event seq: current log length.
#[must_use]
pub(crate) fn next_seq(session: &Session) -> u64 {
    session.events().len() as u64
}

/// User-role text message for tests and queue helpers.
#[cfg(test)]
#[must_use]
pub(crate) fn user_text(id: &str, text: &str) -> Message {
    Message {
        id: MessageId::new(id),
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: text.to_string(),
        }],
        source: MessageSource::User,
    }
}

/// Append one event whose `seq`/`time` are the current log length.
///
/// # Errors
///
/// [`LoopError::Session`] when the session rejects the event.
pub(crate) fn append_event(
    session: &mut Session,
    build: impl FnOnce(u64) -> SessionEvent,
) -> Result<(), LoopError> {
    let seq = next_seq(session);
    session.append(build(seq))?;
    Ok(())
}

/// Wire `type` strings of every log event, in order.
#[cfg(test)]
#[must_use]
pub(crate) fn event_types(session: &Session) -> Vec<String> {
    session
        .events()
        .iter()
        .map(LogEvent::event_type)
        .map(str::to_string)
        .collect()
}
