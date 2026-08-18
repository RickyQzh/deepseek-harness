//! Ordered surface fold over message-producing session events.

use std::collections::HashSet;

use crate::error::SessionError;
use crate::event::{LogEvent, SessionEvent, SurfaceOp};
use crate::message::{ContentBlock, Message, ToolResultData};

/// One replacement observed while folding a session surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceFoldReplacement {
    /// Seq of the event that replaced the prior surface range.
    pub seq: u64,
    /// Declared inclusive start seq of the replaced surface range.
    pub start: u64,
    /// Declared inclusive end seq of the replaced surface range.
    pub end: u64,
    /// Surface entries removed by the operation, in surface order.
    pub shadowed_seqs: Vec<u64>,
}

/// Detached sequences and replacement history after replaying a log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceFoldResult {
    /// Current surface event sequences in model-visible order.
    pub nodes: Vec<u64>,
    /// Replacement operations in event order.
    pub replacements: Vec<SurfaceFoldReplacement>,
}

#[derive(Clone, Debug, Default)]
struct SurfaceFoldState {
    nodes: Vec<u64>,
    replace_generation: u64,
}

/// A validated surface transition that has not mutated fold state yet.
#[derive(Clone, Debug)]
pub(crate) enum SurfacePlan {
    Append {
        seq: u64,
    },
    Replace {
        seq: u64,
        start: u64,
        end: u64,
        start_idx: usize,
        end_idx: usize,
        shadowed_seqs: Vec<u64>,
    },
}

/// Incremental ordered surface view and append-boundary validator.
#[derive(Clone, Debug, Default)]
pub(crate) struct SurfaceManager {
    state: SurfaceFoldState,
}

impl SurfaceManager {
    /// Validate the next known event without mutating the committed surface.
    pub(crate) fn validate_next(
        &self,
        event: &SessionEvent,
        log: &[LogEvent],
        expected_seq: u64,
    ) -> Result<Option<SurfacePlan>, SessionError> {
        plan_surface_event(&self.state, event, expected_seq, |seq| {
            match log.get(seq as usize) {
                Some(LogEvent::Known(event)) => Some(event),
                _ => None,
            }
        })
    }

    /// Commit one previously validated surface transition.
    pub(crate) fn apply(&mut self, plan: Option<SurfacePlan>) {
        apply_surface_plan(&mut self.state, plan);
    }

    pub(crate) fn nodes(&self) -> &[u64] {
        &self.state.nodes
    }

    pub(crate) fn replace_generation(&self) -> u64 {
        self.state.replace_generation
    }
}

/// Replay a complete known-event log through the canonical surface fold.
///
/// # Errors
///
/// [`SessionError::Append`] when `seq` is not the event index.
/// [`SessionError::Surface`] when a marker, provenance, range, or tool-result rewrite rule fails.
pub fn fold_surface(events: &[SessionEvent]) -> Result<SurfaceFoldResult, SessionError> {
    let mut state = SurfaceFoldState::default();
    let mut replacements = Vec::new();
    for (index, event) in events.iter().enumerate() {
        let plan = plan_surface_event(&state, event, index as u64, |seq| events.get(seq as usize))?;
        if let Some(replacement) = apply_surface_plan(&mut state, plan) {
            replacements.push(replacement);
        }
    }
    Ok(SurfaceFoldResult {
        nodes: state.nodes,
        replacements,
    })
}

/// Project one event to the LLM message it derives, or [`None`] when it produces none.
#[must_use]
pub fn derive_event_message(event: &SessionEvent) -> Option<Message> {
    match event {
        SessionEvent::UserMessage { data, .. } => Some(data.clone()),
        SessionEvent::AssistantMessage { data, .. } => {
            if data.message.content.is_empty() {
                None
            } else {
                Some(data.message.clone())
            }
        }
        SessionEvent::ToolResult { data, .. } => Some(data.message.clone()),
        _ => None,
    }
}

fn surface_op_of(event: &SessionEvent) -> Result<Option<&SurfaceOp>, SessionError> {
    if !SessionEvent::is_surface_eligible_type(event.event_type()) {
        return Ok(None);
    }
    match event.surface_op() {
        Some(op) => Ok(Some(op)),
        None => Err(SessionError::Surface(format!(
            "session event \"{}\" is surface-eligible and requires a surfaceOp marker",
            event.event_type()
        ))),
    }
}

fn assert_provenance(event: &SessionEvent, shadowed_seqs: &[u64]) -> Result<(), SessionError> {
    let mut sources = HashSet::new();
    if let Some(raw) = event.source_event_seqs() {
        if raw.is_empty() && event.event_type() != "assistant/message" {
            return Err(SessionError::Surface(
                "sourceEventSeqs must not be empty except on assistant/message".into(),
            ));
        }
        let mut non_earlier = None;
        for &source in raw {
            if !sources.insert(source) {
                return Err(SessionError::Surface(
                    "sourceEventSeqs must not contain duplicates".into(),
                ));
            }
            if non_earlier.is_none() && source >= event.seq() {
                non_earlier = Some(source);
            }
        }
        if let Some(source) = non_earlier {
            return Err(SessionError::Surface(format!(
                "sourceEventSeqs must reference earlier events: {source} >= current seq {}",
                event.seq()
            )));
        }
    }
    let missing: Vec<u64> = shadowed_seqs
        .iter()
        .copied()
        .filter(|seq| !sources.contains(seq))
        .collect();
    if !missing.is_empty() {
        let missing = missing
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(SessionError::Surface(format!(
            "surface replace: sourceEventSeqs must include every shadowed surface node; missing {missing}"
        )));
    }
    Ok(())
}

fn replacement_range(
    state: &SurfaceFoldState,
    start: u64,
    end: u64,
) -> Result<(usize, usize, Vec<u64>), SessionError> {
    let start_idx = state
        .nodes
        .iter()
        .position(|&seq| seq == start)
        .ok_or_else(|| {
            SessionError::Surface(format!(
                "surface replace: start seq {start} not found in surface"
            ))
        })?;
    let end_idx = state
        .nodes
        .iter()
        .position(|&seq| seq == end)
        .ok_or_else(|| {
            SessionError::Surface(format!(
                "surface replace: end seq {end} not found in surface"
            ))
        })?;
    if start_idx > end_idx {
        return Err(SessionError::Surface(format!(
            "surface replace: start seq {start} (index {start_idx}) is after end seq {end} (index {end_idx})"
        )));
    }
    Ok((
        start_idx,
        end_idx,
        state.nodes[start_idx..=end_idx].to_vec(),
    ))
}

fn neutralize_first_block_content(data: &mut ToolResultData) {
    if let Some(ContentBlock::ToolResult { content, .. }) = data.message.content.first_mut() {
        content.clear();
    }
}

fn tool_result_rewrite_rest_equal(original: &ToolResultData, replacement: &ToolResultData) -> bool {
    let mut original_rest = original.clone();
    let mut replacement_rest = replacement.clone();
    neutralize_first_block_content(&mut original_rest);
    neutralize_first_block_content(&mut replacement_rest);
    original_rest == replacement_rest
}

fn assert_tool_result_rewrite<'a>(
    event: &SessionEvent,
    shadowed_seqs: &[u64],
    lookup: impl Fn(u64) -> Option<&'a SessionEvent>,
) -> Result<(), SessionError> {
    let SessionEvent::ToolResult {
        data: replacement, ..
    } = event
    else {
        return Ok(());
    };
    if shadowed_seqs.len() != 1 {
        return Err(SessionError::Surface(
            "tool/result surface replacement must rewrite exactly one current node".into(),
        ));
    }
    for &original_seq in shadowed_seqs {
        let Some(SessionEvent::ToolResult { data: original, .. }) = lookup(original_seq) else {
            return Err(SessionError::Surface(
                "tool/result surface replacement must target a current tool/result".into(),
            ));
        };
        if !tool_result_rewrite_rest_equal(original, replacement) {
            return Err(SessionError::Surface(
                "tool/result surface replacement may change only content".into(),
            ));
        }
    }
    Ok(())
}

fn plan_surface_event<'a>(
    state: &SurfaceFoldState,
    event: &SessionEvent,
    expected_seq: u64,
    lookup: impl Fn(u64) -> Option<&'a SessionEvent>,
) -> Result<Option<SurfacePlan>, SessionError> {
    if event.seq() != expected_seq {
        return Err(SessionError::Append(format!(
            "session event seq {} is not contiguous; expected {expected_seq}",
            event.seq()
        )));
    }
    let Some(surface_op) = surface_op_of(event)? else {
        return Ok(None);
    };
    match surface_op {
        SurfaceOp::Append => {
            assert_provenance(event, &[])?;
            Ok(Some(SurfacePlan::Append { seq: event.seq() }))
        }
        SurfaceOp::Replace { start, end } => {
            let start = *start;
            let end = *end;
            let (start_idx, end_idx, shadowed_seqs) = replacement_range(state, start, end)?;
            assert_provenance(event, &shadowed_seqs)?;
            assert_tool_result_rewrite(event, &shadowed_seqs, lookup)?;
            Ok(Some(SurfacePlan::Replace {
                seq: event.seq(),
                start,
                end,
                start_idx,
                end_idx,
                shadowed_seqs,
            }))
        }
    }
}

fn apply_surface_plan(
    state: &mut SurfaceFoldState,
    plan: Option<SurfacePlan>,
) -> Option<SurfaceFoldReplacement> {
    match plan {
        Some(SurfacePlan::Append { seq }) => {
            state.nodes.push(seq);
            None
        }
        Some(SurfacePlan::Replace {
            seq,
            start,
            end,
            start_idx,
            end_idx,
            shadowed_seqs,
        }) => {
            state.nodes.splice(start_idx..=end_idx, [seq]);
            state.replace_generation += 1;
            Some(SurfaceFoldReplacement {
                seq,
                start,
                end,
                shadowed_seqs,
            })
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{derive_event_message, fold_surface};
    use crate::message::{
        AssistantMessageData, ContentBlock, Message, MessageRole, MessageSource, ToolResultData,
        TurnStartData,
    };
    use crate::{CallId, MessageId, SessionEvent, SurfaceOp};

    fn user(seq: u64, text: &str) -> SessionEvent {
        SessionEvent::UserMessage {
            seq,
            time: seq as i64,
            data: Message {
                id: MessageId::new(format!("m{seq}")),
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
    fn append_nodes_stay_in_order() {
        let events = vec![
            SessionEvent::TurnStart {
                seq: 0,
                time: 0,
                data: TurnStartData { turn: 1 },
                ignorable: None,
            },
            user(1, "hello"),
        ];
        let folded = fold_surface(&events).expect("fold");
        assert_eq!(folded.nodes, vec![1]);
        assert!(folded.replacements.is_empty());
    }

    #[test]
    fn replace_shadows_range_and_bumps_generation() {
        let events = vec![
            user(0, "a"),
            user(1, "b"),
            SessionEvent::UserMessage {
                seq: 2,
                time: 2,
                data: Message {
                    id: MessageId::new("m2"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text {
                        text: "summary".into(),
                    }],
                    source: MessageSource::User,
                },
                surface_op: Some(SurfaceOp::Replace { start: 0, end: 1 }),
                source_event_seqs: Some(vec![0, 1]),
                ignorable: None,
            },
        ];
        let folded = fold_surface(&events).expect("fold");
        assert_eq!(folded.nodes, vec![2]);
        assert_eq!(folded.replacements.len(), 1);
        assert_eq!(folded.replacements[0].shadowed_seqs, vec![0, 1]);
    }

    #[test]
    fn empty_assistant_content_derives_nothing() {
        let event = SessionEvent::AssistantMessage {
            seq: 0,
            time: 0,
            data: AssistantMessageData {
                turn: 1,
                step: 1,
                message: Message {
                    id: MessageId::new("a"),
                    role: MessageRole::Assistant,
                    content: vec![],
                    source: MessageSource::Model {
                        provider: "mock".into(),
                        model: "mock".into(),
                        replay_state: None,
                    },
                },
                usage: None,
            },
            surface_op: Some(SurfaceOp::Append),
            source_event_seqs: Some(vec![]),
            ignorable: None,
        };
        assert!(derive_event_message(&event).is_none());
    }

    #[test]
    fn replace_missing_start_fails() {
        let events = vec![
            user(0, "a"),
            SessionEvent::UserMessage {
                seq: 1,
                time: 1,
                data: Message {
                    id: MessageId::new("m1"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text { text: "x".into() }],
                    source: MessageSource::User,
                },
                surface_op: Some(SurfaceOp::Replace { start: 9, end: 9 }),
                source_event_seqs: Some(vec![9]),
                ignorable: None,
            },
        ];
        let error = fold_surface(&events).expect_err("missing start");
        assert!(error.to_string().contains("start seq 9 not found"));
    }

    fn tool_result(
        seq: u64,
        call_id: &str,
        surface_op: SurfaceOp,
        source_event_seqs: Option<Vec<u64>>,
    ) -> SessionEvent {
        SessionEvent::ToolResult {
            seq,
            time: seq as i64,
            data: ToolResultData {
                turn: 1,
                step: 1,
                message: Message {
                    id: MessageId::new(format!("tr{seq}")),
                    role: MessageRole::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_call_id: CallId::new(call_id),
                        content: vec![ContentBlock::Text {
                            text: format!("result {seq}"),
                        }],
                        is_error: Some(false),
                    }],
                    source: MessageSource::Tool {
                        call_id: CallId::new(call_id),
                    },
                },
                error: None,
                meta: None,
            },
            surface_op: Some(surface_op),
            source_event_seqs,
            ignorable: None,
        }
    }

    #[test]
    fn replace_source_event_seqs_must_include_every_shadowed_node() {
        let events = vec![
            user(0, "a"),
            user(1, "b"),
            SessionEvent::UserMessage {
                seq: 2,
                time: 2,
                data: Message {
                    id: MessageId::new("m2"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text {
                        text: "summary".into(),
                    }],
                    source: MessageSource::User,
                },
                surface_op: Some(SurfaceOp::Replace { start: 0, end: 1 }),
                source_event_seqs: Some(vec![0]),
                ignorable: None,
            },
        ];
        let error = fold_surface(&events).expect_err("incomplete provenance");
        assert!(
            error
                .to_string()
                .contains("sourceEventSeqs must include every shadowed surface node")
        );
    }

    #[test]
    fn surface_eligible_event_without_marker_fails() {
        let events = vec![SessionEvent::UserMessage {
            seq: 0,
            time: 0,
            data: Message {
                id: MessageId::new("m0"),
                role: MessageRole::User,
                content: vec![ContentBlock::Text {
                    text: "hidden".into(),
                }],
                source: MessageSource::User,
            },
            surface_op: None,
            source_event_seqs: None,
            ignorable: None,
        }];
        let error = fold_surface(&events).expect_err("missing marker");
        assert!(error.to_string().contains("requires a surfaceOp marker"));
    }

    #[test]
    fn tool_result_replace_must_rewrite_exactly_one_current_node() {
        let events = vec![
            user(0, "a"),
            user(1, "b"),
            tool_result(
                2,
                "rewrite",
                SurfaceOp::Replace { start: 0, end: 1 },
                Some(vec![0, 1]),
            ),
        ];
        let error = fold_surface(&events).expect_err("multi-node tool rewrite");
        assert!(
            error
                .to_string()
                .contains("must rewrite exactly one current node")
        );
    }

    /// Same rest fields across seqs so only inner `content` or an explicit rest patch differs.
    fn rewriteable_tool_result(
        seq: u64,
        call_id: &str,
        content_text: &str,
        is_error: Option<bool>,
        meta: Option<serde_json::Value>,
        surface_op: SurfaceOp,
        source_event_seqs: Option<Vec<u64>>,
    ) -> SessionEvent {
        SessionEvent::ToolResult {
            seq,
            time: seq as i64,
            data: ToolResultData {
                turn: 1,
                step: 1,
                message: Message {
                    id: MessageId::new("tr-stable"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_call_id: CallId::new(call_id),
                        content: vec![ContentBlock::Text {
                            text: content_text.into(),
                        }],
                        is_error,
                    }],
                    source: MessageSource::Tool {
                        call_id: CallId::new(call_id),
                    },
                },
                error: None,
                meta,
            },
            surface_op: Some(surface_op),
            source_event_seqs,
            ignorable: None,
        }
    }

    fn tool_result_rewrite_pair(replacement_content: &str) -> (SessionEvent, SessionEvent) {
        (
            rewriteable_tool_result(0, "call", "old", Some(false), None, SurfaceOp::Append, None),
            rewriteable_tool_result(
                1,
                "call",
                replacement_content,
                Some(false),
                None,
                SurfaceOp::Replace { start: 0, end: 0 },
                Some(vec![0]),
            ),
        )
    }

    #[test]
    fn tool_result_replace_must_target_a_current_tool_result() {
        let events = vec![
            user(0, "a"),
            tool_result(
                1,
                "rewrite",
                SurfaceOp::Replace { start: 0, end: 0 },
                Some(vec![0]),
            ),
        ];
        let error = fold_surface(&events).expect_err("non-result target");
        assert!(
            error
                .to_string()
                .contains("must target a current tool/result")
        );
    }

    fn assert_tool_result_rest_drift(patch: impl FnOnce(&mut ToolResultData)) {
        let (original, mut replacement) = tool_result_rewrite_pair("old");
        let SessionEvent::ToolResult { data, .. } = &mut replacement else {
            panic!("replacement is a tool/result");
        };
        patch(data);
        let error = fold_surface(&[original, replacement]).expect_err("rest-field drift");
        assert!(error.to_string().contains("may change only content"));
    }

    #[test]
    fn tool_result_replace_may_change_only_content() {
        assert_tool_result_rest_drift(|data| {
            if let Some(ContentBlock::ToolResult { tool_call_id, .. }) =
                data.message.content.first_mut()
            {
                *tool_call_id = CallId::new("other");
            }
        });
        assert_tool_result_rest_drift(|data| {
            if let Some(ContentBlock::ToolResult { is_error, .. }) =
                data.message.content.first_mut()
            {
                *is_error = Some(true);
            }
        });
        assert_tool_result_rest_drift(|data| {
            data.meta = Some(serde_json::json!({ "k": 1 }));
        });
    }

    #[test]
    fn tool_result_replace_accepts_content_only_rewrite() {
        let (original, replacement) = tool_result_rewrite_pair("rewritten");
        let folded = fold_surface(&[original, replacement]).expect("content-only rewrite");
        assert_eq!(folded.nodes, vec![1]);
        assert_eq!(folded.replacements.len(), 1);
        assert_eq!(folded.replacements[0].shadowed_seqs, vec![0]);
    }
}
