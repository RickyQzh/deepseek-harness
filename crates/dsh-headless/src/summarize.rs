//! Last assistant text and `turn/end` reason from an owned seq interval.

use dsh_session::{ContentBlock, LogEvent, SessionEvent, TurnEndReason};

/// Outcome of one owned run interval.
pub struct RunOutcome {
    /// Last non-empty joined assistant text blocks.
    pub text: String,
    /// Last `turn/end` reason in the interval, if any.
    pub reason: Option<TurnEndReason>,
}

/// Port of `packages/bundle/headless/src/index.ts` `summarize`.
#[must_use]
pub fn summarize(events: &[LogEvent], first_seq: u64) -> RunOutcome {
    let mut started = false;
    let mut text = String::new();
    let mut reason = None;
    for event in events {
        let LogEvent::Known(event) = event else {
            continue;
        };
        if event.seq() < first_seq {
            continue;
        }
        match event {
            SessionEvent::TurnStart { .. } => {
                started = true;
            }
            SessionEvent::AssistantMessage { data, .. } if started => {
                let joined: String = data
                    .message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                if !joined.is_empty() {
                    text = joined;
                }
            }
            SessionEvent::TurnEnd { data, .. } if started => {
                reason = Some(data.reason.clone());
            }
            _ => {}
        }
    }
    RunOutcome { text, reason }
}

#[cfg(test)]
mod tests {
    use super::summarize;
    use dsh_session::{
        AssistantMessageData, ContentBlock, LogEvent, Message, MessageId, MessageRole,
        MessageSource, SessionEvent, SurfaceOp, TurnEndData, TurnEndReason, TurnStartData,
    };

    fn text_message(text: &str) -> Message {
        Message {
            id: MessageId::new("a1"),
            role: MessageRole::Assistant,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            source: MessageSource::Model {
                provider: "mock".into(),
                model: "mock".into(),
                replay_state: None,
            },
        }
    }

    #[test]
    fn summarize_takes_last_non_empty_assistant_text_after_first_seq() {
        let events = vec![
            LogEvent::Known(SessionEvent::TurnStart {
                seq: 0,
                time: 0,
                data: TurnStartData { turn: 1 },
                ignorable: None,
            }),
            LogEvent::Known(SessionEvent::AssistantMessage {
                seq: 1,
                time: 1,
                data: AssistantMessageData {
                    turn: 1,
                    step: 1,
                    message: text_message("ignored-before"),
                    usage: None,
                },
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            }),
            LogEvent::Known(SessionEvent::TurnStart {
                seq: 5,
                time: 5,
                data: TurnStartData { turn: 2 },
                ignorable: None,
            }),
            LogEvent::Known(SessionEvent::AssistantMessage {
                seq: 6,
                time: 6,
                data: AssistantMessageData {
                    turn: 2,
                    step: 1,
                    message: text_message("hello"),
                    usage: None,
                },
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            }),
            LogEvent::Known(SessionEvent::AssistantMessage {
                seq: 7,
                time: 7,
                data: AssistantMessageData {
                    turn: 2,
                    step: 1,
                    message: text_message(""),
                    usage: None,
                },
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            }),
            LogEvent::Known(SessionEvent::AssistantMessage {
                seq: 8,
                time: 8,
                data: AssistantMessageData {
                    turn: 2,
                    step: 1,
                    message: text_message("final-text"),
                    usage: None,
                },
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            }),
            LogEvent::Known(SessionEvent::TurnEnd {
                seq: 9,
                time: 9,
                data: TurnEndData {
                    turn: 2,
                    reason: TurnEndReason::Completed,
                },
                ignorable: None,
            }),
        ];
        let outcome = summarize(&events, 5);
        assert_eq!(outcome.text, "final-text");
        assert!(matches!(outcome.reason, Some(TurnEndReason::Completed)));
    }
}
