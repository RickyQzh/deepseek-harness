//! In-memory session: header, log, and incremental surface.

use std::sync::Arc;

use crate::error::SessionError;
use crate::event::{LogEvent, SessionEvent};
use crate::header::SessionHeader;
use crate::ids::SessionId;
use crate::message::{EpochHeader, Message};
use crate::request_header::fold_request_header_iter;
use crate::surface::{SurfaceManager, derive_event_message};

/// Observer invoked after a known event is admitted (`seq` already assigned).
pub type AppendSink = Arc<dyn Fn(&LogEvent) + Send + Sync>;

/// Append-only session log with an incremental message-producing surface.
#[derive(Clone)]
pub struct Session {
    header: SessionHeader,
    events: Vec<LogEvent>,
    surface: SurfaceManager,
    append_sink: Option<AppendSink>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("header", &self.header)
            .field("events", &self.events)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Empty session with `header` and no events.
    #[must_use]
    pub fn new(header: SessionHeader) -> Self {
        Self {
            header,
            events: Vec::new(),
            surface: SurfaceManager::default(),
            append_sink: None,
        }
    }

    /// Install or clear the append observer. Invoked once per successful [`Session::append`].
    pub fn set_append_sink(&mut self, sink: Option<AppendSink>) {
        self.append_sink = sink;
    }

    /// Restore a session by validating each known event incrementally.
    ///
    /// Leftovers stay in the log and do not join the surface. Every event's
    /// `seq` must equal its index.
    ///
    /// # Errors
    ///
    /// [`SessionError::Append`] when `seq` is not the event index.
    /// [`SessionError::Surface`] when a known event fails the surface fold.
    pub fn from_events(header: SessionHeader, events: Vec<LogEvent>) -> Result<Self, SessionError> {
        let mut session = Self::new(header);
        for (index, event) in events.into_iter().enumerate() {
            let expected = index as u64;
            if event.seq() != expected {
                return Err(SessionError::Append(format!(
                    "session event seq {} is not contiguous; expected {expected}",
                    event.seq()
                )));
            }
            match event {
                LogEvent::Known(known) => {
                    let plan = session
                        .surface
                        .validate_next(&known, &session.events, expected)?;
                    session.events.push(LogEvent::Known(known));
                    session.surface.apply(plan);
                }
                leftover @ LogEvent::Leftover(_) => session.events.push(leftover),
            }
        }
        Ok(session)
    }

    /// Session identity from the durable header.
    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.header.id
    }

    /// Durable storage metadata kept outside the event log.
    #[must_use]
    pub fn header(&self) -> &SessionHeader {
        &self.header
    }

    /// Append-only log, including leftovers.
    #[must_use]
    pub fn events(&self) -> &[LogEvent] {
        &self.events
    }

    /// Admit one known event at `seq == events.len()`.
    ///
    /// # Errors
    ///
    /// [`SessionError::Append`] when `event.seq()` is not the next log index.
    /// [`SessionError::Surface`] when the event fails the surface fold.
    pub fn append(&mut self, event: SessionEvent) -> Result<&SessionEvent, SessionError> {
        let expected = self.events.len() as u64;
        if event.seq() != expected {
            return Err(SessionError::Append(format!(
                "session event seq {} is not contiguous; expected {expected}",
                event.seq()
            )));
        }
        let plan = self.surface.validate_next(&event, &self.events, expected)?;
        self.events.push(LogEvent::Known(event));
        self.surface.apply(plan);
        if let Some(sink) = &self.append_sink {
            if let Some(LogEvent::Known(_)) = self.events.last() {
                sink(self.events.last().expect("pushed"));
            }
        }
        match self.events.last() {
            Some(LogEvent::Known(event)) => Ok(event),
            Some(LogEvent::Leftover(_)) | None => unreachable!("append pushes Known"),
        }
    }

    /// Walk surface nodes and project each to a message, skipping empty projections.
    #[must_use]
    pub fn derive_messages(&self) -> Vec<Message> {
        let mut out = Vec::new();
        for seq in self.surface.nodes() {
            let Some(LogEvent::Known(event)) = self.events.get(*seq as usize) else {
                continue;
            };
            if let Some(message) = derive_event_message(event) {
                out.push(message);
            }
        }
        out
    }

    /// Current surface event sequences in model-visible order.
    #[must_use]
    pub fn surface_nodes(&self) -> &[u64] {
        self.surface.nodes()
    }

    /// Monotonic count of committed positional replacements.
    #[must_use]
    pub fn replace_generation(&self) -> u64 {
        self.surface.replace_generation()
    }

    /// Fold known `request/header` events into the header in force after the last snapshot.
    ///
    /// Leftovers are skipped. Returns `None` before the first header event.
    #[must_use]
    pub fn request_header(&self) -> Option<EpochHeader> {
        fold_request_header_iter(
            self.events.iter().filter_map(|event| match event {
                LogEvent::Known(event) => Some(event),
                LogEvent::Leftover(_) => None,
            }),
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Session;
    use crate::message::{ContentBlock, Message, MessageRole, MessageSource, TurnStartData};
    use crate::{
        LogEvent, SESSION_FORMAT_VERSION, SessionEvent, SessionHeader, SessionId, SurfaceOp,
    };

    fn header() -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("ss"),
            created_at: 1,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        }
    }

    fn user(seq: u64, text: &str) -> SessionEvent {
        SessionEvent::UserMessage {
            seq,
            time: seq as i64,
            data: Message {
                id: crate::MessageId::new(format!("m{seq}")),
                role: MessageRole::User,
                content: vec![ContentBlock::Text { text: text.into() }],
                source: MessageSource::User,
            },
            surface_op: Some(SurfaceOp::Append),
            source_event_seqs: None,
            ignorable: None,
        }
    }

    #[test]
    fn derive_messages_follows_surface_not_raw_log() {
        let mut session = Session::new(header());
        session
            .append(SessionEvent::TurnStart {
                seq: 0,
                time: 0,
                data: TurnStartData { turn: 1 },
                ignorable: None,
            })
            .unwrap();
        session.append(user(1, "hello")).unwrap();
        let messages = session.derive_messages();
        assert_eq!(messages.len(), 1);
        assert!(
            matches!(messages[0].content[0], ContentBlock::Text { ref text } if text == "hello")
        );
    }

    #[test]
    fn leftover_does_not_join_surface() {
        let leftover = crate::LeftoverEvent {
            type_name: "future/event".into(),
            seq: 0,
            time: 1,
            data: serde_json::json!({"payload": 1}),
            ignorable: Some(true),
            surface_op: None,
            source_event_seqs: None,
        };
        let session = Session::from_events(header(), vec![LogEvent::Leftover(leftover)]).unwrap();
        assert!(session.surface_nodes().is_empty());
        assert!(session.derive_messages().is_empty());
    }

    #[test]
    fn append_rejects_non_contiguous_seq() {
        let mut session = Session::new(header());
        let error = session.append(user(3, "x")).expect_err("seq");
        assert!(error.to_string().contains("not contiguous"));
    }

    #[test]
    fn append_sink_observes_known_events() {
        use std::sync::{Arc, Mutex};
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut session = Session::new(header());
        let seen_clone = Arc::clone(&seen);
        session.set_append_sink(Some(Arc::new(move |event| {
            seen_clone
                .lock()
                .expect("seen")
                .push(event.event_type().to_string());
        })));
        session.append(user(0, "hello")).unwrap();
        assert_eq!(
            *seen.lock().expect("seen"),
            vec!["user/message".to_string()]
        );
    }

    #[test]
    fn append_replace_bumps_replace_generation() {
        let mut session = Session::new(header());
        session.append(user(0, "a")).unwrap();
        session.append(user(1, "b")).unwrap();
        let before = session.replace_generation();
        session
            .append(SessionEvent::UserMessage {
                seq: 2,
                time: 2,
                data: Message {
                    id: crate::MessageId::new("m2"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text {
                        text: "summary".into(),
                    }],
                    source: MessageSource::User,
                },
                surface_op: Some(SurfaceOp::Replace { start: 0, end: 1 }),
                source_event_seqs: Some(vec![0, 1]),
                ignorable: None,
            })
            .unwrap();
        assert!(session.replace_generation() > before);
    }
}
