//! Crash-recovery closers for an interrupted session-log tail.

use crate::message::{
    ContentBlock, Message, MessageRole, MessageSource, StepBoundaryData, ToolResultData,
    ToolResultError, TurnEndData, TurnEndReason,
};
use crate::{CallId, MessageId, SessionEvent, SurfaceOp};

/// Recovery code for an assistant tool request that never reached a recorded call start.
pub const TOOL_NOT_STARTED: &str = "TOOL_NOT_STARTED";

/// Recovery code for a recorded tool call whose completed outcome was not durably recorded.
pub const TOOL_OUTCOME_UNKNOWN: &str = "TOOL_OUTCOME_UNKNOWN";

const NOT_STARTED_TEXT: &str = "The tool call was interrupted before the Harness recorded it as started. Retry it if it is still needed.";

const OUTCOME_UNKNOWN_TEXT: &str = "The tool call was interrupted after it was recorded, but no result was durably recorded. Its outcome is unknown. Decide whether to retry from the tool semantics: retry only if the operation is read-only or idempotent; if it may have side effects, first verify external state or ask the user. Do not retry blindly.";

struct PendingCall {
    step: u64,
    call_seq: Option<u64>,
}

/// Return deterministic synthetic events that close an open tail turn.
///
/// Unmatched calls receive error results first, then an open `step/end` and an
/// interrupted `turn/end`. Sequences continue the log; timestamps reuse the last
/// real event. A balanced or empty log returns no events.
///
/// # Parameters
///
/// * `events` - loaded durable log to scan (a valid committed prefix, possibly with a crash tail).
///
/// # Returns
///
/// Synthetic closer events to append after `events`, in order; empty when the log is already balanced.
#[must_use]
pub fn interrupted_turn_closers(events: &[SessionEvent]) -> Vec<SessionEvent> {
    let mut open_turn = None;
    let mut open_step = None;
    let mut pending_calls: Vec<(CallId, PendingCall)> = Vec::new();

    for event in events {
        match event {
            SessionEvent::TurnStart { data, .. } => {
                open_turn = Some(data.turn);
                open_step = None;
                pending_calls.clear();
            }
            SessionEvent::TurnEnd { .. } => {
                open_turn = None;
                open_step = None;
                pending_calls.clear();
            }
            SessionEvent::StepStart { data, .. } => {
                open_step = Some(data.step);
            }
            SessionEvent::StepEnd { .. } => {
                pending_calls.clear();
                open_step = None;
            }
            SessionEvent::AssistantMessage { data, .. } => {
                for block in &data.message.content {
                    if let ContentBlock::ToolCall { id, .. } = block {
                        upsert_pending(&mut pending_calls, id.clone(), data.step);
                    }
                }
            }
            SessionEvent::ToolCall { seq, data, .. } => {
                if let Some(entry) = pending_mut(&mut pending_calls, &data.call_id) {
                    entry.call_seq = Some(*seq);
                }
            }
            SessionEvent::ToolResult { data, .. } => {
                if let MessageSource::Tool { call_id } = &data.message.source {
                    pending_calls.retain(|(id, _)| id != call_id);
                }
            }
            _ => {}
        }
    }

    let Some(last) = events.last() else {
        return Vec::new();
    };
    let Some(open_turn) = open_turn else {
        return Vec::new();
    };

    let mut seq = last.seq() + 1;
    let time = last.time();
    let mut closers = Vec::new();

    for (call_id, pending) in pending_calls {
        closers.push(synthetic_tool_result(
            seq,
            time,
            open_turn,
            pending.step,
            &call_id,
            pending.call_seq,
        ));
        seq += 1;
    }

    if let Some(step) = open_step {
        closers.push(SessionEvent::StepEnd {
            seq,
            time,
            data: StepBoundaryData {
                turn: open_turn,
                step,
            },
            ignorable: None,
        });
        seq += 1;
    }

    closers.push(SessionEvent::TurnEnd {
        seq,
        time,
        data: TurnEndData {
            turn: open_turn,
            reason: TurnEndReason::Interrupted,
        },
        ignorable: None,
    });
    closers
}

fn pending_mut<'a>(
    pending: &'a mut [(CallId, PendingCall)],
    call_id: &CallId,
) -> Option<&'a mut PendingCall> {
    pending
        .iter_mut()
        .find(|(id, _)| id == call_id)
        .map(|(_, entry)| entry)
}

fn upsert_pending(pending: &mut Vec<(CallId, PendingCall)>, call_id: CallId, step: u64) {
    if let Some(entry) = pending_mut(pending, &call_id) {
        *entry = PendingCall {
            step,
            call_seq: None,
        };
        return;
    }
    pending.push((
        call_id,
        PendingCall {
            step,
            call_seq: None,
        },
    ));
}

fn synthetic_tool_result(
    seq: u64,
    time: i64,
    turn: u64,
    step: u64,
    call_id: &CallId,
    call_seq: Option<u64>,
) -> SessionEvent {
    let started = call_seq.is_some();
    let (error_name, error_code, text) = if started {
        (
            "ToolOutcomeUnknownError",
            TOOL_OUTCOME_UNKNOWN,
            OUTCOME_UNKNOWN_TEXT,
        )
    } else {
        ("ToolNotStartedError", TOOL_NOT_STARTED, NOT_STARTED_TEXT)
    };
    SessionEvent::ToolResult {
        seq,
        time,
        data: ToolResultData {
            turn,
            step,
            message: Message {
                id: MessageId::new(format!(
                    "interrupted-tool-result-{}-{seq}",
                    call_id.as_str()
                )),
                role: MessageRole::User,
                content: vec![ContentBlock::ToolResult {
                    tool_call_id: call_id.clone(),
                    content: vec![ContentBlock::Text {
                        text: text.to_owned(),
                    }],
                    is_error: Some(true),
                }],
                source: MessageSource::Tool {
                    call_id: call_id.clone(),
                },
            },
            error: Some(ToolResultError {
                name: error_name.to_owned(),
                code: error_code.to_owned(),
            }),
            meta: None,
        },
        surface_op: Some(SurfaceOp::Append),
        source_event_seqs: call_seq.map(|call_seq| vec![call_seq]),
        ignorable: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{TOOL_NOT_STARTED, TOOL_OUTCOME_UNKNOWN, interrupted_turn_closers};
    use crate::message::{
        AssistantMessageData, ContentBlock, Message, MessageRole, MessageSource, StepBoundaryData,
        ToolCallData, ToolResultData, TurnEndData, TurnEndReason, TurnStartData,
    };
    use crate::{CallId, MessageId, SessionEvent, SurfaceOp};

    fn turn_start(turn: u64, seq: u64) -> SessionEvent {
        SessionEvent::TurnStart {
            seq,
            time: seq as i64,
            data: TurnStartData { turn },
            ignorable: None,
        }
    }

    fn assistant_with_call(seq: u64, turn: u64, step: u64, call: &str) -> SessionEvent {
        SessionEvent::AssistantMessage {
            seq,
            time: seq as i64,
            data: AssistantMessageData {
                turn,
                step,
                message: Message {
                    id: MessageId::new("a"),
                    role: MessageRole::Assistant,
                    content: vec![ContentBlock::ToolCall {
                        id: CallId::new(call),
                        name: "bash".into(),
                        arguments: "{}".into(),
                    }],
                    source: MessageSource::Model {
                        provider: "mock".into(),
                        model: "mock".into(),
                        replay_state: None,
                    },
                },
                usage: None,
            },
            surface_op: Some(SurfaceOp::Append),
            source_event_seqs: None,
            ignorable: None,
        }
    }

    #[test]
    fn balanced_log_returns_nothing() {
        let events = vec![
            turn_start(1, 0),
            SessionEvent::TurnEnd {
                seq: 1,
                time: 1,
                data: TurnEndData {
                    turn: 1,
                    reason: TurnEndReason::Completed,
                },
                ignorable: None,
            },
        ];
        assert!(interrupted_turn_closers(&events).is_empty());
    }

    #[test]
    fn empty_log_returns_nothing() {
        assert!(interrupted_turn_closers(&[]).is_empty());
    }

    #[test]
    fn open_turn_without_step_closes_turn_only() {
        let closers = interrupted_turn_closers(&[turn_start(1, 0)]);
        assert_eq!(closers.len(), 1);
        assert_eq!(closers[0].event_type(), "turn/end");
        assert_eq!(closers[0].seq(), 1);
    }

    #[test]
    fn open_step_closes_step_then_turn() {
        let events = vec![
            turn_start(1, 0),
            SessionEvent::StepStart {
                seq: 1,
                time: 1,
                data: StepBoundaryData { turn: 1, step: 1 },
                ignorable: None,
            },
        ];
        let closers = interrupted_turn_closers(&events);
        assert_eq!(
            closers
                .iter()
                .map(SessionEvent::event_type)
                .collect::<Vec<_>>(),
            ["step/end", "turn/end"]
        );
        assert_eq!(
            closers.iter().map(SessionEvent::seq).collect::<Vec<_>>(),
            [2, 3]
        );
    }

    #[test]
    fn unanswered_assistant_call_is_not_started() {
        let events = vec![
            turn_start(2, 0),
            SessionEvent::StepStart {
                seq: 1,
                time: 1,
                data: StepBoundaryData { turn: 2, step: 1 },
                ignorable: None,
            },
            assistant_with_call(2, 2, 1, "call-1"),
        ];
        let closers = interrupted_turn_closers(&events);
        assert_eq!(
            closers
                .iter()
                .map(SessionEvent::event_type)
                .collect::<Vec<_>>(),
            ["tool/result", "step/end", "turn/end"]
        );
        match &closers[0] {
            SessionEvent::ToolResult { data, .. } => {
                assert_eq!(
                    data.error.as_ref().map(|error| error.code.as_str()),
                    Some(TOOL_NOT_STARTED)
                );
                let ContentBlock::ToolResult {
                    content, is_error, ..
                } = &data.message.content[0]
                else {
                    panic!("tool-result block");
                };
                assert_eq!(*is_error, Some(true));
                assert!(
                    matches!(&content[0], ContentBlock::Text { text } if text.contains("before the Harness recorded it as started"))
                );
            }
            _ => panic!("tool/result"),
        }
    }

    #[test]
    fn recorded_call_without_result_is_outcome_unknown() {
        let events = vec![
            turn_start(2, 0),
            SessionEvent::StepStart {
                seq: 1,
                time: 1,
                data: StepBoundaryData { turn: 2, step: 1 },
                ignorable: None,
            },
            assistant_with_call(2, 2, 1, "call-1"),
            SessionEvent::ToolCall {
                seq: 3,
                time: 3,
                data: ToolCallData {
                    turn: 2,
                    step: 1,
                    call_id: CallId::new("call-1"),
                    name: "bash".into(),
                    arguments: "{}".into(),
                },
                ignorable: None,
            },
        ];
        let closers = interrupted_turn_closers(&events);
        match &closers[0] {
            SessionEvent::ToolResult {
                data,
                source_event_seqs,
                ..
            } => {
                assert_eq!(
                    data.error.as_ref().map(|error| error.code.as_str()),
                    Some(TOOL_OUTCOME_UNKNOWN)
                );
                assert_eq!(source_event_seqs.as_deref(), Some(&[3][..]));
            }
            _ => panic!("tool/result"),
        }
    }

    #[test]
    fn answered_call_does_not_synthesize_result() {
        let events = vec![
            turn_start(2, 0),
            SessionEvent::StepStart {
                seq: 1,
                time: 1,
                data: StepBoundaryData { turn: 2, step: 1 },
                ignorable: None,
            },
            assistant_with_call(2, 2, 1, "call-1"),
            SessionEvent::ToolResult {
                seq: 3,
                time: 3,
                data: ToolResultData {
                    turn: 2,
                    step: 1,
                    message: Message {
                        id: MessageId::new("r"),
                        role: MessageRole::User,
                        content: vec![ContentBlock::ToolResult {
                            tool_call_id: CallId::new("call-1"),
                            content: vec![ContentBlock::Text { text: "ok".into() }],
                            is_error: Some(false),
                        }],
                        source: MessageSource::Tool {
                            call_id: CallId::new("call-1"),
                        },
                    },
                    error: None,
                    meta: None,
                },
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            },
        ];
        let types: Vec<_> = interrupted_turn_closers(&events)
            .iter()
            .map(SessionEvent::event_type)
            .collect();
        assert_eq!(types, ["step/end", "turn/end"]);
    }
}
