//! Last retained runtime-context snapshot, without committing user messages.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use dsh_session::{
    ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, Session, SessionEvent,
    SurfaceOp,
};
use dsh_system_prompt::ContextSnapshotSection;

/// Plugin source stamped on runtime-context snapshot user messages.
pub const RUNTIME_CONTEXT_SOURCE: &str = "@deepseek-ai/dsh-system-prompt";
/// Model-visible marker when dynamic context becomes empty after a prior snapshot.
pub const RUNTIME_CONTEXT_CLEARED: &str =
    "Current runtime context: none. Earlier runtime-context snapshots no longer apply.";

static NEXT_SNAPSHOT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
enum Retained {
    Never,
    Cleared,
    Snapshot { seq: u64, text: Option<String> },
}

/// Tracks the last retained runtime-context snapshot without owning its commit.
pub struct RuntimeContextProjection {
    retained: Retained,
}

impl RuntimeContextProjection {
    /// Restore retained snapshot state by walking the log backwards for the last owned surface user message.
    #[must_use]
    pub fn replay(session: &Session) -> Self {
        let surface: HashSet<u64> = session.surface_nodes().iter().copied().collect();
        let mut retained = Retained::Never;
        for event in session.events().iter().rev() {
            let LogEvent::Known(SessionEvent::UserMessage { seq, data, .. }) = event else {
                continue;
            };
            if !is_owned(data) {
                continue;
            }
            if matches!(retained, Retained::Never) {
                retained = Retained::Cleared;
            }
            if surface.contains(seq) {
                retained = Retained::Snapshot {
                    seq: *seq,
                    text: text_of(data),
                };
                break;
            }
        }
        Self { retained }
    }

    /// Update retained state from a later `user/message` or a replacement that cites the retained seq.
    pub fn observe(&mut self, event: &SessionEvent) {
        if let SessionEvent::UserMessage { data, .. } = event {
            if is_owned(data) {
                self.retained = Retained::Snapshot {
                    seq: event.seq(),
                    text: text_of(data),
                };
                return;
            }
        }
        let Retained::Snapshot { seq, .. } = self.retained else {
            return;
        };
        if matches!(event.surface_op(), Some(SurfaceOp::Replace { .. }))
            && event
                .source_event_seqs()
                .is_some_and(|seqs| seqs.contains(&seq))
        {
            self.retained = Retained::Cleared;
        }
    }

    /// Create an uncommitted snapshot user message only when the retained text differs.
    ///
    /// First empty current with no prior snapshot returns [`None`]. Empty current after a
    /// retained snapshot yields the cleared marker.
    #[must_use]
    pub fn project(&self, current: &str, sections: &[ContextSnapshotSection]) -> Option<Message> {
        if matches!(self.retained, Retained::Never) && current.is_empty() {
            return None;
        }
        let snapshot = if current.is_empty() {
            RUNTIME_CONTEXT_CLEARED.to_string()
        } else {
            current.to_string()
        };
        if let Retained::Snapshot {
            text: Some(text), ..
        } = &self.retained
        {
            if text == &snapshot {
                return None;
            }
        }
        Some(Message {
            id: MessageId::new(format!(
                "runtime-context-{}",
                NEXT_SNAPSHOT_ID.fetch_add(1, Ordering::Relaxed)
            )),
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text: snapshot }],
            source: plugin_source(sections),
        })
    }
}

fn is_owned(message: &Message) -> bool {
    matches!(
        &message.source,
        MessageSource::Plugin { plugin, .. } if plugin == RUNTIME_CONTEXT_SOURCE
    )
}

fn text_of(message: &Message) -> Option<String> {
    match message.content.as_slice() {
        [ContentBlock::Text { text }] => Some(text.clone()),
        _ => None,
    }
}

fn plugin_source(sections: &[ContextSnapshotSection]) -> MessageSource {
    if sections.is_empty() {
        return MessageSource::Plugin {
            plugin: RUNTIME_CONTEXT_SOURCE.into(),
            form: None,
            sections: Vec::new(),
            summary: None,
        };
    }
    let value = serde_json::json!({
        "kind": "plugin",
        "plugin": RUNTIME_CONTEXT_SOURCE,
        "form": "snapshot",
        "sections": sections.iter().map(|section| {
            serde_json::json!({
                "name": section.name,
                "text": section.text,
            })
        }).collect::<Vec<_>>(),
    });
    serde_json::from_value(value).unwrap_or(MessageSource::Plugin {
        plugin: RUNTIME_CONTEXT_SOURCE.into(),
        form: Some("snapshot".into()),
        sections: Vec::new(),
        summary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::RuntimeContextProjection;
    use crate::{RUNTIME_CONTEXT_CLEARED, RUNTIME_CONTEXT_SOURCE, test_header};
    use dsh_session::{
        ContentBlock, Message, MessageId, MessageRole, MessageSource, Session, SessionEvent,
        SurfaceOp,
    };

    #[test]
    fn project_none_then_identity_then_cleared_marker() {
        let session = Session::new(test_header("rt-1"));
        let mut projection = RuntimeContextProjection::replay(&session);
        assert!(projection.project("", &[]).is_none());

        let snapshot = SessionEvent::UserMessage {
            seq: 0,
            time: 0,
            data: Message {
                id: MessageId::new("snap-1"),
                role: MessageRole::User,
                content: vec![ContentBlock::Text {
                    text: "cwd=/tmp".into(),
                }],
                source: MessageSource::Plugin {
                    plugin: RUNTIME_CONTEXT_SOURCE.into(),
                    form: Some("snapshot".into()),
                    sections: vec![],
                    summary: None,
                },
            },
            surface_op: Some(SurfaceOp::Append),
            source_event_seqs: None,
            ignorable: None,
        };
        projection.observe(&snapshot);
        assert!(projection.project("cwd=/tmp", &[]).is_none());
        let cleared = projection.project("", &[]).expect("cleared marker");
        match &cleared.content[..] {
            [ContentBlock::Text { text }] => assert_eq!(text, RUNTIME_CONTEXT_CLEARED),
            other => panic!("expected cleared text, got {other:?}"),
        }
    }
}
