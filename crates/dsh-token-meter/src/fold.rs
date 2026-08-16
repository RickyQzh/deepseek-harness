//! Per-session priced surface fold and `TokenMeter` service.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use dsh_llm::BlockAssembler;
use dsh_session::{
    EpochHeader, LogEvent, Message, Session, SessionEvent, SurfaceOp, TokenUsage, canonical_header,
    derive_event_message, header_equals,
};

use crate::{ROLE_OVERHEAD, estimate_content, estimate_header, estimate_message};

/// One token-priced node in the current ordered session surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenSurfaceNode {
    seq: u64,
    tokens: u64,
}

impl TokenSurfaceNode {
    /// Durable sequence number of the surface event.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Heuristic tokens for the exact message projected by this node.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.tokens
    }
}

/// The baseline from which a signed surface delta produces current pressure.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenMeasurementBaseline {
    /// No request envelope and an empty surface.
    None,
    /// Full heuristic price of the measured header plus surface.
    Estimated {
        /// Heuristic tokens at the anchor or current reprice.
        tokens: u64,
    },
    /// Provider-reported usage reused for an identical canonical header.
    Usage {
        /// Sum of disjoint usage buckets.
        tokens: u64,
        /// Provider usage payload that produced `tokens`.
        usage: TokenUsage,
    },
}

impl TokenMeasurementBaseline {
    /// Tokens this baseline contributes before the signed surface delta.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        match self {
            Self::None => 0,
            Self::Estimated { tokens } | Self::Usage { tokens, .. } => *tokens,
        }
    }

    /// Provider usage when this baseline is [`Self::Usage`].
    #[must_use]
    pub fn usage(&self) -> Option<&TokenUsage> {
        match self {
            Self::Usage { usage, .. } => Some(usage),
            Self::None | Self::Estimated { .. } => None,
        }
    }
}

/// Detached request-pressure and surface snapshot at one consumed log revision.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenMeasurement {
    log_revision: u64,
    baseline: TokenMeasurementBaseline,
    surface_delta_tokens: i64,
    total_tokens: u64,
    surface_tokens: u64,
    nodes: Vec<TokenSurfaceNode>,
}

impl TokenMeasurement {
    /// Number of durable events consumed; equal to the next unread event seq.
    #[must_use]
    pub fn log_revision(&self) -> u64 {
        self.log_revision
    }

    /// Provider or heuristic anchor used for this measurement.
    #[must_use]
    pub fn baseline(&self) -> &TokenMeasurementBaseline {
        &self.baseline
    }

    /// Signed repricing of current surface content relative to the baseline anchor.
    #[must_use]
    pub fn surface_delta_tokens(&self) -> i64 {
        self.surface_delta_tokens
    }

    /// Non-negative current request-and-response pressure.
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.total_tokens
    }

    /// Total heuristic tokens across the current surface.
    #[must_use]
    pub fn surface_tokens(&self) -> u64 {
        self.surface_tokens
    }

    /// Current surface nodes in positional head-to-tail order.
    #[must_use]
    pub fn nodes(&self) -> &[TokenSurfaceNode] {
        &self.nodes
    }
}

#[derive(Clone, Debug)]
struct StepStartState {
    turn: u64,
    step: u64,
    surface_tokens: u64,
}

#[derive(Clone, Debug)]
struct MeasurementAnchor {
    header: Option<EpochHeader>,
    surface_tokens: u64,
    baseline: TokenMeasurementBaseline,
}

#[derive(Clone, Debug, Default)]
struct ReplayState {
    consumed_events: u64,
    header: Option<EpochHeader>,
    surface: Vec<TokenSurfaceNode>,
    surface_tokens: u64,
    step_start: Option<StepStartState>,
    anchor: Option<MeasurementAnchor>,
}

struct SurfaceTokenFold {
    tokens: u64,
    nodes: Vec<TokenSurfaceNode>,
    delta_tokens: i64,
}

/// Replay owner for one service-wide estimator and isolated per-session folds.
#[derive(Default)]
pub struct TokenMeter {
    states: Mutex<HashMap<String, ReplayState>>,
}

impl TokenMeter {
    /// Empty meter with no per-session fold state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
        }
    }

    /// Measure current request pressure and surface through the durable tail.
    ///
    /// Provider usage is reused only when the latest successful call's canonical
    /// request envelope matches `request_header` and its total is no lower than
    /// that call's full heuristic anchor; otherwise the complete envelope and
    /// surface are heuristically repriced.
    ///
    /// `request_header` affects request pressure only; surface fields always
    /// describe the current session surface.
    ///
    /// # Parameters
    ///
    /// * `session` - session to replay through its current durable tail.
    /// * `request_header` - optional effective request envelope replacing the latest logged header.
    ///
    /// # Returns
    ///
    /// A detached pressure and surface measurement at one consumed-log revision.
    ///
    /// # Panics
    ///
    /// Panics when the durable log has an unmatched step boundary, an assistant
    /// message without a matching `step/start`, a cited chunk that is not an
    /// earlier `assistant/chunk` of the same step, or a replace range absent
    /// from the priced surface.
    #[must_use]
    pub fn measure(
        &self,
        session: &Session,
        request_header: Option<&EpochHeader>,
    ) -> TokenMeasurement {
        let mut states = self.states.lock().expect("token meter fold");
        let key = session.id().as_str().to_owned();
        let event_len = session.events().len() as u64;
        let state = states.entry(key).or_default();
        if event_len < state.consumed_events {
            *state = ReplayState::default();
        }
        while state.consumed_events < event_len {
            let index = usize::try_from(state.consumed_events).expect("event index");
            match session.events().get(index) {
                Some(LogEvent::Known(event)) => fold_event(session, state, event),
                Some(LogEvent::Leftover(_)) | None => {}
            }
            state.consumed_events += 1;
        }
        snapshot(state, request_header)
    }

    /// Heuristically price one model-visible message.
    ///
    /// # Parameters
    ///
    /// * `message` - message to price without mutation.
    ///
    /// # Returns
    ///
    /// Content and role-framing tokens under the fixed service heuristic.
    #[must_use]
    pub fn estimate_message(&self, message: &Message) -> u64 {
        estimate_message(message)
    }
}

fn snapshot(state: &ReplayState, request_header: Option<&EpochHeader>) -> TokenMeasurement {
    let header = match request_header {
        Some(header) => Some(canonical_header(header)),
        None => state.header.clone(),
    };
    let matching = match &state.anchor {
        Some(anchor) => optional_header_equals(anchor.header.as_ref(), header.as_ref()),
        None => false,
    };
    let (baseline, surface_delta_tokens) = if matching {
        let anchor = state.anchor.as_ref().expect("matching implies anchor");
        (
            anchor.baseline.clone(),
            i64::try_from(state.surface_tokens).expect("surface tokens")
                - i64::try_from(anchor.surface_tokens).expect("anchor surface tokens"),
        )
    } else if header.is_none() && state.surface_tokens == 0 {
        (TokenMeasurementBaseline::None, 0)
    } else {
        (
            TokenMeasurementBaseline::Estimated {
                tokens: estimate_header(header.as_ref()) + state.surface_tokens,
            },
            0,
        )
    };
    TokenMeasurement {
        log_revision: state.consumed_events,
        total_tokens: baseline
            .tokens()
            .saturating_add_signed(surface_delta_tokens),
        baseline,
        surface_delta_tokens,
        surface_tokens: state.surface_tokens,
        nodes: state.surface.clone(),
    }
}

fn optional_header_equals(left: Option<&EpochHeader>, right: Option<&EpochHeader>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => header_equals(left, right),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn usage_tokens(usage: &TokenUsage) -> u64 {
    usage.input_tokens
        + usage.cache_read_tokens.unwrap_or(0)
        + usage.cache_write_tokens.unwrap_or(0)
        + usage.output_tokens
}

fn fold_surface_tokens(nodes: &[TokenSurfaceNode], event: &SessionEvent) -> SurfaceTokenFold {
    let tokens = match derive_event_message(event) {
        Some(message) => estimate_message(&message),
        None => 0,
    };
    match event.surface_op() {
        Some(SurfaceOp::Append) => {
            let mut next = nodes.to_vec();
            next.push(TokenSurfaceNode {
                seq: event.seq(),
                tokens,
            });
            SurfaceTokenFold {
                tokens,
                nodes: next,
                delta_tokens: i64::try_from(tokens).expect("event tokens"),
            }
        }
        Some(SurfaceOp::Replace { start, end }) => {
            let start = *start;
            let end = *end;
            let start_idx = nodes.iter().position(|node| node.seq == start);
            let end_idx = nodes.iter().position(|node| node.seq == end);
            match (start_idx, end_idx) {
                (Some(start_idx), Some(end_idx)) if start_idx <= end_idx => {
                    let removed: u64 = nodes[start_idx..=end_idx]
                        .iter()
                        .map(|node| node.tokens)
                        .sum();
                    let mut next = nodes.to_vec();
                    next.splice(
                        start_idx..=end_idx,
                        [TokenSurfaceNode {
                            seq: event.seq(),
                            tokens,
                        }],
                    );
                    SurfaceTokenFold {
                        tokens,
                        nodes: next,
                        delta_tokens: i64::try_from(tokens).expect("event tokens")
                            - i64::try_from(removed).expect("removed tokens"),
                    }
                }
                _ => panic!(
                    "token surface: replace at seq {} has invalid current range {start}-{end}",
                    event.seq()
                ),
            }
        }
        None => panic!("token surface: missing surfaceOp at seq {}", event.seq()),
    }
}

fn fold_event(session: &Session, state: &mut ReplayState, event: &SessionEvent) {
    let mut next_header = state.header.clone();
    let mut next_step_start = state.step_start.clone();
    let mut next_anchor = state.anchor.clone();

    match event {
        SessionEvent::RequestHeader { data, .. } => {
            next_header = Some(canonical_header(&data.header));
        }
        SessionEvent::StepStart { data, seq, .. } => {
            if let Some(start) = &state.step_start {
                panic!(
                    "token meter: step/start at seq {seq} arrived before turn {}/step {} ended",
                    start.turn, start.step
                );
            }
            next_step_start = Some(StepStartState {
                turn: data.turn,
                step: data.step,
                surface_tokens: state.surface_tokens,
            });
        }
        SessionEvent::StepEnd { data, seq, .. } => {
            let matched = match &state.step_start {
                Some(start) => start.turn == data.turn && start.step == data.step,
                None => false,
            };
            if !matched {
                panic!("token meter: step/end at seq {seq} has no matching step/start event");
            }
            next_step_start = None;
        }
        _ => {}
    }

    let surface = match event.surface_op() {
        Some(_) => Some(fold_surface_tokens(&state.surface, event)),
        None => None,
    };

    if let SessionEvent::AssistantMessage { data, seq, .. } = event {
        let step_start = match &state.step_start {
            Some(start) if start.turn == data.turn && start.step == data.step => start,
            Some(_) | None => panic!(
                "token meter: assistant/message at seq {seq} has no matching step/start event"
            ),
        };
        let event_tokens = match &surface {
            Some(fold) => fold.tokens,
            None => {
                panic!("token meter: assistant/message at seq {seq} is missing a surfaceOp marker")
            }
        };
        if data.usage.is_some() && next_header.is_some() {
            let usage = data.usage.as_ref().expect("checked");
            let provider_assistant_tokens =
                estimate_provider_assistant(session, event, event_tokens);
            let anchor_surface_tokens = step_start.surface_tokens + provider_assistant_tokens;
            let provider_tokens = usage_tokens(usage);
            let estimated_anchor_tokens =
                estimate_header(next_header.as_ref()) + anchor_surface_tokens;
            let baseline = if provider_tokens >= estimated_anchor_tokens {
                TokenMeasurementBaseline::Usage {
                    tokens: provider_tokens,
                    usage: usage.clone(),
                }
            } else {
                TokenMeasurementBaseline::Estimated {
                    tokens: estimated_anchor_tokens,
                }
            };
            next_anchor = Some(MeasurementAnchor {
                header: next_header.clone(),
                surface_tokens: anchor_surface_tokens,
                baseline,
            });
        } else {
            let anchor_surface_tokens = step_start.surface_tokens + event_tokens;
            next_anchor = Some(MeasurementAnchor {
                header: next_header.clone(),
                surface_tokens: anchor_surface_tokens,
                baseline: TokenMeasurementBaseline::Estimated {
                    tokens: estimate_header(next_header.as_ref()) + anchor_surface_tokens,
                },
            });
        }
    }

    state.header = next_header;
    state.step_start = next_step_start;
    if let Some(surface) = surface {
        state.surface = surface.nodes;
        state.surface_tokens = state
            .surface_tokens
            .saturating_add_signed(surface.delta_tokens);
    }
    state.anchor = next_anchor;
}

fn estimate_provider_assistant(
    session: &Session,
    event: &SessionEvent,
    durable_event_tokens: u64,
) -> u64 {
    let Some(source_seqs) = event.source_event_seqs() else {
        return durable_event_tokens;
    };
    let (turn, step) = match event {
        SessionEvent::AssistantMessage { data, .. } => (data.turn, data.step),
        _ => panic!("token meter: provider assistant estimate requires assistant/message"),
    };
    let mut assembler = BlockAssembler::new();
    let mut seen = HashSet::new();
    for &seq in source_seqs {
        if seq >= event.seq() {
            panic!(
                "token meter: assistant/message at seq {} source seq {seq} is not earlier",
                event.seq()
            );
        }
        if !seen.insert(seq) {
            panic!(
                "token meter: assistant/message at seq {} repeats source seq {seq}",
                event.seq()
            );
        }
        let source_event = match session
            .events()
            .get(usize::try_from(seq).expect("source seq"))
        {
            Some(LogEvent::Known(source_event)) => source_event,
            Some(LogEvent::Leftover(_)) | None => panic!(
                "token meter: assistant/message at seq {} source seq {seq} is not assistant/chunk",
                event.seq()
            ),
        };
        match source_event {
            SessionEvent::AssistantChunk { data, .. } => {
                if data.turn != turn || data.step != step {
                    panic!(
                        "token meter: assistant/message at seq {} source seq {seq} belongs to another step",
                        event.seq()
                    );
                }
                assembler.push(data.chunk.clone());
            }
            _ => panic!(
                "token meter: assistant/message at seq {} source seq {seq} is not assistant/chunk",
                event.seq()
            ),
        }
    }
    let provider_content = assembler.blocks();
    if provider_content.is_empty() {
        0
    } else {
        estimate_content(&provider_content) + u64::from(ROLE_OVERHEAD)
    }
}

#[cfg(test)]
mod tests {
    use super::{TokenMeasurementBaseline, TokenMeter};
    use crate::{estimate_header, estimate_message};
    use dsh_session::{
        AssistantChunkData, AssistantMessageData, ContentBlock, EpochHeader, FinishReason,
        LlmCallConfig, Message, MessageId, MessageRole, MessageSource, RequestHeaderData,
        RequestHeaderReason, SESSION_FORMAT_VERSION, Session, SessionEvent, SessionHeader,
        SessionId, StepBoundaryData, StreamChunk, SurfaceOp, TokenUsage, canonical_header,
    };
    use serde_json::json;

    fn user_text(text: &str) -> Message {
        Message {
            id: MessageId::new("m"),
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            source: MessageSource::User,
        }
    }

    fn user_message(seq: u64, text: &str) -> SessionEvent {
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

    fn session_header(id: &str) -> SessionHeader {
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

    fn session_with_two_user_messages(first: &str, second: &str) -> Session {
        let mut session = Session::new(session_header("meter-replace"));
        session.append(user_message(0, first)).unwrap();
        session.append(user_message(1, second)).unwrap();
        session
    }

    fn replace_first_range(session: &mut Session) {
        let seq = session.events().len() as u64;
        session
            .append(SessionEvent::UserMessage {
                seq,
                time: seq as i64,
                data: Message {
                    id: MessageId::new(format!("m{seq}")),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text { text: "x".into() }],
                    source: MessageSource::User,
                },
                surface_op: Some(SurfaceOp::Replace { start: 0, end: 0 }),
                source_event_seqs: Some(vec![0]),
                ignorable: None,
            })
            .unwrap();
    }

    #[derive(Clone, Copy)]
    enum CallProvenance {
        Exact,
        Empty,
        Absent,
    }

    fn call_config(provider: &str, model: &str) -> LlmCallConfig {
        LlmCallConfig {
            provider: provider.into(),
            model: model.into(),
            reasoning_effort: None,
            temperature: None,
            max_tokens: None,
            stop: None,
        }
    }

    fn epoch_header(model: &str, system: Option<&str>) -> EpochHeader {
        EpochHeader {
            config: call_config("mock", model),
            adapter_defaults: None,
            system: system.map(str::to_owned),
            tools: None,
        }
    }

    fn disjoint_usage() -> TokenUsage {
        TokenUsage {
            input_tokens: 20,
            output_tokens: 7,
            cache_read_tokens: Some(3),
            cache_write_tokens: Some(4),
            reasoning_tokens: Some(6),
        }
    }

    fn assistant_text(text: &str) -> Message {
        Message {
            id: MessageId::new("a"),
            role: MessageRole::Assistant,
            content: if text.is_empty() {
                vec![]
            } else {
                vec![ContentBlock::Text { text: text.into() }]
            },
            source: MessageSource::Model {
                provider: "mock".into(),
                model: "m".into(),
                replay_state: None,
            },
        }
    }

    fn next_seq(session: &Session) -> u64 {
        session.events().len() as u64
    }

    fn append(session: &mut Session, event: SessionEvent) {
        session.append(event).expect("session append");
    }

    fn append_successful_call(
        session: &mut Session,
        header: EpochHeader,
        provider_text: &str,
        durable_text: &str,
        usage: Option<TokenUsage>,
        provenance: CallProvenance,
    ) {
        let turn = 1;
        let step = 1;
        let seq = next_seq(session);
        append(
            session,
            SessionEvent::StepStart {
                seq,
                time: seq as i64,
                data: StepBoundaryData { turn, step },
                ignorable: None,
            },
        );
        let seq = next_seq(session);
        append(
            session,
            SessionEvent::RequestHeader {
                seq,
                time: seq as i64,
                data: RequestHeaderData {
                    header,
                    reason: RequestHeaderReason::Initial,
                },
                ignorable: None,
            },
        );

        let mut sources = Vec::new();
        if matches!(provenance, CallProvenance::Exact) {
            let chunks = [
                StreamChunk::BlockStart {
                    index: 0,
                    block_type: "text".into(),
                },
                StreamChunk::TextDelta {
                    index: 0,
                    text: provider_text.into(),
                },
                StreamChunk::BlockEnd {
                    index: 0,
                    block: ContentBlock::Text {
                        text: provider_text.into(),
                    },
                },
                StreamChunk::Finish {
                    reason: FinishReason::Stop,
                    replay_state: None,
                },
            ];
            for chunk in chunks {
                let seq = next_seq(session);
                sources.push(seq);
                append(
                    session,
                    SessionEvent::AssistantChunk {
                        seq,
                        time: seq as i64,
                        data: AssistantChunkData { turn, step, chunk },
                        ignorable: None,
                    },
                );
            }
        }

        let source_event_seqs = match provenance {
            CallProvenance::Exact => Some(sources),
            CallProvenance::Empty => Some(Vec::new()),
            CallProvenance::Absent => None,
        };
        let seq = next_seq(session);
        append(
            session,
            SessionEvent::AssistantMessage {
                seq,
                time: seq as i64,
                data: AssistantMessageData {
                    turn,
                    step,
                    message: assistant_text(durable_text),
                    usage,
                },
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs,
                ignorable: None,
            },
        );
        let seq = next_seq(session);
        append(
            session,
            SessionEvent::StepEnd {
                seq,
                time: seq as i64,
                data: StepBoundaryData { turn, step },
                ignorable: None,
            },
        );
    }

    fn session_with_user_and_call(
        id: &str,
        user: &str,
        header: EpochHeader,
        provider_text: &str,
        durable_text: &str,
        usage: Option<TokenUsage>,
        provenance: CallProvenance,
    ) -> Session {
        let mut session = Session::new(session_header(id));
        if !user.is_empty() {
            append(&mut session, user_message(0, user));
        }
        append_successful_call(
            &mut session,
            header,
            provider_text,
            durable_text,
            usage,
            provenance,
        );
        session
    }

    #[test]
    fn measure_surface_tokens_drop_after_replace() {
        let meter = TokenMeter::new();
        let mut session = session_with_two_user_messages("hello world", "more text here");
        let before = meter.measure(&session, None);
        replace_first_range(&mut session);
        let after = meter.measure(&session, None);
        assert!(after.surface_tokens() < before.surface_tokens());
        assert!(after.log_revision() > before.log_revision());
    }

    #[test]
    fn measure_empty_session_has_none_baseline() {
        let meter = TokenMeter::new();
        let session = Session::new(session_header("meter-empty"));
        let measured = meter.measure(&session, None);
        assert!(matches!(
            measured.baseline(),
            super::TokenMeasurementBaseline::None
        ));
        assert_eq!(measured.total_tokens(), 0);
        assert_eq!(measured.surface_tokens(), 0);
        assert!(measured.nodes().is_empty());
        assert_eq!(measured.log_revision(), 0);
    }

    #[test]
    fn measure_without_header_estimates_surface_only() {
        let meter = TokenMeter::new();
        let session = session_with_two_user_messages("abcd", "ab");
        let measured = meter.measure(&session, None);
        let expected = estimate_message(&user_text("abcd")) + estimate_message(&user_text("ab"));
        assert_eq!(measured.surface_tokens(), expected);
        assert_eq!(measured.total_tokens(), expected);
        assert_eq!(
            measured
                .nodes()
                .iter()
                .map(super::TokenSurfaceNode::tokens)
                .sum::<u64>(),
            measured.surface_tokens()
        );
        assert!(matches!(
            measured.baseline(),
            super::TokenMeasurementBaseline::Estimated { .. }
        ));
    }

    #[test]
    fn measure_usage_baseline_when_disjoint_buckets_meet_estimate() {
        let header = epoch_header("deepseek-v4-flash", None);
        let user = "before";
        let provider_text = "short";
        let durable_text = "a much longer rewritten durable assistant answer";
        let estimated_anchor = estimate_header(Some(&header))
            + estimate_message(&user_text(user))
            + estimate_message(&assistant_text(provider_text));
        let usage = TokenUsage {
            input_tokens: estimated_anchor,
            output_tokens: 0,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: Some(99),
        };
        let session = session_with_user_and_call(
            "usage-eq",
            user,
            header.clone(),
            provider_text,
            durable_text,
            Some(usage.clone()),
            CallProvenance::Exact,
        );
        let meter = TokenMeter::new();
        let measured = meter.measure(&session, None);
        match measured.baseline() {
            TokenMeasurementBaseline::Usage { tokens, usage: got } => {
                assert_eq!(*tokens, estimated_anchor);
                assert_eq!(got, &usage);
            }
            other => panic!("expected usage baseline, got {other:?}"),
        }
        assert!(measured.surface_delta_tokens() > 0);
        assert_eq!(
            measured.total_tokens(),
            estimated_anchor.saturating_add_signed(measured.surface_delta_tokens())
        );

        let matching = meter.measure(&session, Some(&canonical_header(&header)));
        assert_eq!(matching.baseline(), measured.baseline());
        assert_eq!(
            matching.surface_delta_tokens(),
            measured.surface_delta_tokens()
        );
    }

    #[test]
    fn measure_usage_tokens_do_not_count_reasoning_twice() {
        let usage = disjoint_usage();
        let session = session_with_user_and_call(
            "usage-reasoning",
            "before",
            epoch_header("deepseek-v4-flash", None),
            "short",
            "a much longer rewritten durable assistant answer",
            Some(usage.clone()),
            CallProvenance::Exact,
        );
        let measured = TokenMeter::new().measure(&session, None);
        match measured.baseline() {
            TokenMeasurementBaseline::Usage { tokens, usage: got } => {
                assert_eq!(*tokens, 20 + 3 + 4 + 7);
                assert_eq!(got.reasoning_tokens, Some(6));
                assert_eq!(got, &usage);
            }
            other => panic!("expected usage baseline, got {other:?}"),
        }
    }

    #[test]
    fn measure_empty_source_event_seqs_zero_provider_absent_uses_durable() {
        let usage = disjoint_usage();
        let header = epoch_header("deepseek-v4-flash", None);
        let durable = "listener injected text";
        let empty = session_with_user_and_call(
            "explicit-empty",
            "",
            header.clone(),
            "",
            durable,
            Some(usage.clone()),
            CallProvenance::Empty,
        );
        let absent = session_with_user_and_call(
            "legacy-absent",
            "",
            header,
            "",
            durable,
            Some(usage),
            CallProvenance::Absent,
        );
        let meter = TokenMeter::new();
        let empty_measured = meter.measure(&empty, None);
        let absent_measured = meter.measure(&absent, None);
        assert!(matches!(
            empty_measured.baseline(),
            TokenMeasurementBaseline::Usage { tokens: 34, .. }
        ));
        assert!(matches!(
            absent_measured.baseline(),
            TokenMeasurementBaseline::Usage { tokens: 34, .. }
        ));
        // Explicit [] prices a known-empty provider stream; omitted seqs use durable output.
        assert!(empty_measured.surface_delta_tokens() > 0);
        assert_eq!(absent_measured.surface_delta_tokens(), 0);
    }

    #[test]
    fn measure_mismatched_request_header_reprices_instead_of_usage_anchor() {
        let header = epoch_header("deepseek-v4-flash", Some("one"));
        let session = session_with_user_and_call(
            "envelope",
            "before",
            header.clone(),
            "short",
            "short",
            Some(disjoint_usage()),
            CallProvenance::Exact,
        );
        let meter = TokenMeter::new();
        let anchored = meter.measure(&session, None);
        assert!(matches!(
            anchored.baseline(),
            TokenMeasurementBaseline::Usage { tokens: 34, .. }
        ));
        let surface_tokens = anchored.surface_tokens();

        let mut empty_tools = header.clone();
        empty_tools.tools = Some(json!([]));
        let with_empty_tools = meter.measure(&session, Some(&empty_tools));
        assert_eq!(with_empty_tools.baseline(), anchored.baseline());
        assert_eq!(
            with_empty_tools.surface_delta_tokens(),
            anchored.surface_delta_tokens()
        );

        let mut provider = header.clone();
        provider.config.provider = "other".into();
        let mut model = header.clone();
        model.config.model = "deepseek-v4-pro".into();
        let mut system = header.clone();
        system.system = Some("two".into());
        let mut tools = header.clone();
        tools.tools = Some(json!([{ "name": "read", "description": "read" }]));

        for override_header in [provider, model, system, tools] {
            let repriced = meter.measure(&session, Some(&override_header));
            match repriced.baseline() {
                TokenMeasurementBaseline::Estimated { tokens } => {
                    assert_eq!(
                        *tokens,
                        estimate_header(Some(&canonical_header(&override_header))) + surface_tokens
                    );
                }
                other => panic!("expected estimated reprice, got {other:?}"),
            }
            assert_eq!(repriced.surface_delta_tokens(), 0);
            assert_eq!(repriced.surface_tokens(), surface_tokens);
        }
    }
}
