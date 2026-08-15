//! Durable inbox projection over `agent/inbox/spliced` events.

use std::collections::HashSet;

use dsh_session::{
    InboxSplicedData, InboxTarget, LogEvent, Message, Session, SessionEvent, SessionHeader,
};

use crate::error::LoopError;

/// Replay-once projection of the pending next-turn and next-step lists.
pub struct Inbox {
    next_turn: Vec<Message>,
    next_step: Vec<Message>,
}

impl Inbox {
    /// Restore pending lists from durable `agent/inbox/spliced` events after `seed_length`.
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] when a persisted splice does not apply to the reconstructed lists.
    pub fn replay(session: &Session) -> Result<Self, LoopError> {
        let mut inbox = Self {
            next_turn: Vec::new(),
            next_step: Vec::new(),
        };
        let seed = seed_length(session.header());
        for event in session.events().iter().skip(seed) {
            let LogEvent::Known(SessionEvent::AgentInboxSpliced { seq, data, .. }) = event else {
                continue;
            };
            inbox.apply(data).map_err(|error| {
                LoopError::Invalid(format!(
                    "invalid persisted inbox splice at session seq {seq}: {error}"
                ))
            })?;
        }
        Ok(inbox)
    }

    /// Prompts awaiting a new turn.
    #[must_use]
    pub fn next_turn(&self) -> &[Message] {
        &self.next_turn
    }

    /// Input awaiting the next step boundary.
    #[must_use]
    pub fn next_step(&self) -> &[Message] {
        &self.next_step
    }

    /// Whether either pending list contains work.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.next_turn.is_empty() || !self.next_step.is_empty()
    }

    /// Apply standard splice semantics and append `agent/inbox/spliced` before mutating live lists.
    ///
    /// `removed_count` is omitted when the normalized delete count is 0.
    /// `outcome` is `"canceled"` when `discard_removed` is true and at least one message is removed.
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] when the splice coordinates are invalid or a `MessageId` is already pending.
    /// [`LoopError::Session`] when the session rejects the splice event.
    pub fn splice(
        &mut self,
        session: &mut Session,
        target: InboxTarget,
        start: usize,
        delete_count: usize,
        inserted: Vec<Message>,
        discard_removed: bool,
    ) -> Result<Vec<Message>, LoopError> {
        self.mutate(
            session,
            target,
            start,
            delete_count,
            inserted,
            discard_removed,
        )
    }

    /// Remove and return the batch for one step. `next-turn` also consumes one queued turn.
    ///
    /// Durable splices are pure deletions (`discard_removed = false`).
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] or [`LoopError::Session`] from the underlying splices.
    pub fn claim(
        &mut self,
        session: &mut Session,
        target: InboxTarget,
        _turn: u64,
    ) -> Result<Vec<Message>, LoopError> {
        let mut claimed = self.mutate(
            session,
            InboxTarget::NextStep,
            0,
            self.next_step.len(),
            Vec::new(),
            false,
        )?;
        if target == InboxTarget::NextTurn {
            claimed.extend(self.mutate(session, InboxTarget::NextTurn, 0, 1, Vec::new(), false)?);
        }
        Ok(claimed)
    }

    /// Durably cancel all pending input, clearing next-step before next-turn.
    ///
    /// # Errors
    ///
    /// [`LoopError::Invalid`] or [`LoopError::Session`] from the underlying splices.
    pub fn clear(&mut self, session: &mut Session) -> Result<(), LoopError> {
        self.splice(
            session,
            InboxTarget::NextStep,
            0,
            self.next_step.len(),
            Vec::new(),
            true,
        )?;
        self.splice(
            session,
            InboxTarget::NextTurn,
            0,
            self.next_turn.len(),
            Vec::new(),
            true,
        )?;
        Ok(())
    }

    fn mutate(
        &mut self,
        session: &mut Session,
        target: InboxTarget,
        start: usize,
        delete_count: usize,
        inserted: Vec<Message>,
        discard_removed: bool,
    ) -> Result<Vec<Message>, LoopError> {
        let len = self.list(&target).len();
        let actual_start = start.min(len);
        let actual_delete = delete_count.min(len.saturating_sub(actual_start));
        if actual_delete == 0 && inserted.is_empty() {
            return Ok(Vec::new());
        }
        let outcome = if discard_removed && actual_delete > 0 {
            Some("canceled".to_string())
        } else {
            None
        };
        let splice = InboxSplicedData {
            target,
            start: actual_start as u64,
            removed_count: (actual_delete > 0).then_some(actual_delete as u64),
            inserted,
            outcome,
        };
        self.validate(&splice)?;
        let seq = session.events().len() as u64;
        let event = session.append(SessionEvent::AgentInboxSpliced {
            seq,
            time: seq as i64,
            data: splice,
            ignorable: None,
        })?;
        let SessionEvent::AgentInboxSpliced { data, .. } = event else {
            return Err(LoopError::Invalid("expected agent/inbox/spliced".into()));
        };
        let data = data.clone();
        self.apply(&data)
    }

    fn apply(&mut self, splice: &InboxSplicedData) -> Result<Vec<Message>, LoopError> {
        self.validate(splice)?;
        let start = usize::try_from(splice.start).map_err(|_| invalid_splice())?;
        let removed_count =
            usize::try_from(splice.removed_count.unwrap_or(0)).map_err(|_| invalid_splice())?;
        let list = self.list_mut(&splice.target);
        let removed: Vec<Message> = list
            .splice(
                start..start + removed_count,
                splice.inserted.iter().cloned(),
            )
            .collect();
        Ok(removed)
    }

    fn validate(&self, splice: &InboxSplicedData) -> Result<(), LoopError> {
        let list = self.list(&splice.target);
        let start = usize::try_from(splice.start).map_err(|_| invalid_splice())?;
        let removed_count =
            usize::try_from(splice.removed_count.unwrap_or(0)).map_err(|_| invalid_splice())?;
        if start > list.len() || start.saturating_add(removed_count) > list.len() {
            return Err(invalid_splice());
        }
        let mut candidate: Vec<&Message> = list.iter().collect();
        let _removed: Vec<&Message> = candidate
            .splice(start..start + removed_count, splice.inserted.iter())
            .collect();
        let other = match &splice.target {
            InboxTarget::NextTurn => self.next_step.as_slice(),
            InboxTarget::NextStep => self.next_turn.as_slice(),
        };
        let mut ids = HashSet::new();
        let chain: Vec<&Message> = match &splice.target {
            InboxTarget::NextTurn => candidate.into_iter().chain(other.iter()).collect(),
            InboxTarget::NextStep => other.iter().chain(candidate).collect(),
        };
        for message in chain {
            if !ids.insert(message.id.as_str()) {
                return Err(LoopError::Invalid(format!(
                    "message \"{}\" is already pending",
                    message.id.as_str()
                )));
            }
        }
        Ok(())
    }

    fn list(&self, target: &InboxTarget) -> &[Message] {
        match target {
            InboxTarget::NextTurn => &self.next_turn,
            InboxTarget::NextStep => &self.next_step,
        }
    }

    fn list_mut(&mut self, target: &InboxTarget) -> &mut Vec<Message> {
        match target {
            InboxTarget::NextTurn => &mut self.next_turn,
            InboxTarget::NextStep => &mut self.next_step,
        }
    }
}

fn seed_length(header: &SessionHeader) -> usize {
    header
        .seed_length
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn invalid_splice() -> LoopError {
    LoopError::Invalid("invalid inbox splice".into())
}

#[cfg(test)]
mod tests {
    use super::Inbox;
    use crate::{test_header, user_text};
    use dsh_session::{InboxTarget, Session};

    #[test]
    fn splice_then_replay_restores_next_turn() {
        let mut session = Session::new(test_header("inbox-1"));
        let mut inbox = Inbox::replay(&session).unwrap();
        let message = user_text("m1", "hi");
        inbox
            .splice(
                &mut session,
                InboxTarget::NextTurn,
                0,
                0,
                vec![message.clone()],
                true,
            )
            .unwrap();
        let restored = Inbox::replay(&session).unwrap();
        assert_eq!(restored.next_turn(), std::slice::from_ref(&message));
        assert!(restored.next_step().is_empty());
    }
}
