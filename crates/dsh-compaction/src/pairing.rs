//! Tool-pairing balance over a session surface.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use dsh_session::{ContentBlock, LogEvent, Session, SessionEvent};

/// Incremental balance state for one session surface generation.
#[derive(Clone)]
struct BalanceCache {
    generation: u64,
    cut_balanced: Vec<bool>,
    index_by_seq: HashMap<u64, usize>,
    in_progress_tool_calls: i64,
}

fn cache_map() -> &'static Mutex<HashMap<String, BalanceCache>> {
    static CACHE: OnceLock<Mutex<HashMap<String, BalanceCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Return how one surface event changes the in-progress tool-call count.
fn event_delta(event: &SessionEvent) -> i64 {
    match event {
        SessionEvent::AssistantMessage { data, .. } => data
            .message
            .content
            .iter()
            .filter(|block| matches!(block, ContentBlock::ToolCall { .. }))
            .count() as i64,
        SessionEvent::ToolResult { .. } => -1,
        _ => 0,
    }
}

/// Read and validate the event named by a surface sequence.
fn event_for_seq(events: &[LogEvent], seq: u64) -> &SessionEvent {
    match events.get(seq as usize) {
        Some(LogEvent::Known(event)) if event.seq() == seq => event,
        _ => panic!(
            "tool-pairing balance: surface seq {seq} has no matching session event (corrupt surface)"
        ),
    }
}

/// Fold surface sequences not yet in the cache into its balance state.
fn extend_cache(session: &Session, mut cache: BalanceCache, seqs: &[u64]) -> BalanceCache {
    let processed = cache.cut_balanced.len() - 1;
    let tail = &seqs[processed..];
    let events = session.events();
    let mut pending_cuts = Vec::with_capacity(tail.len());
    let mut in_progress_tool_calls = cache.in_progress_tool_calls;
    for &seq in tail {
        in_progress_tool_calls += event_delta(event_for_seq(events, seq));
        if in_progress_tool_calls < 0 {
            panic!(
                "tool-pairing balance: tool/result at surface seq {seq} has no matching tool-call (corrupt surface)"
            );
        }
        pending_cuts.push(in_progress_tool_calls == 0);
    }
    for (offset, &seq) in tail.iter().enumerate() {
        cache.index_by_seq.insert(seq, processed + offset);
    }
    cache.cut_balanced.extend(pending_cuts);
    cache.in_progress_tool_calls = in_progress_tool_calls;
    cache
}

fn empty_cache(generation: u64) -> BalanceCache {
    BalanceCache {
        generation,
        cut_balanced: vec![true],
        index_by_seq: HashMap::new(),
        in_progress_tool_calls: 0,
    }
}

/// Return balance state synchronized with the current session surface.
fn balance_cache(session: &Session) -> BalanceCache {
    let seqs = session.surface_nodes();
    let generation = session.replace_generation();
    let key = session.id().as_str().to_owned();
    let cached = cache_map()
        .lock()
        .expect("tool-pairing cache")
        .get(&key)
        .cloned();

    let next = match cached {
        None => extend_cache(session, empty_cache(generation), seqs),
        Some(cached)
            if cached.generation != generation || cached.cut_balanced.len() - 1 > seqs.len() =>
        {
            extend_cache(session, empty_cache(generation), seqs)
        }
        Some(cached) if cached.cut_balanced.len() - 1 < seqs.len() => {
            extend_cache(session, cached, seqs)
        }
        Some(cached) => return cached,
    };
    cache_map()
        .lock()
        .expect("tool-pairing cache")
        .insert(key, next.clone());
    next
}

/// Balance of the cut at a sequence's position plus offset, rejecting seqs outside current membership.
fn cut_balance(cache: &BalanceCache, seq: u64, offset: usize) -> bool {
    let Some(&index) = cache.index_by_seq.get(&seq) else {
        panic!("tool-pairing balance: surface seq {seq} not found");
    };
    match cache.cut_balanced.get(index + offset) {
        Some(balanced) => *balanced,
        None => panic!("tool-pairing balance: surface seq {seq} not found"),
    }
}

/// Whether the cut immediately before a current surface sequence is tool-pairing balanced.
///
/// # Parameters
///
/// * `session` - session whose surface is checked.
/// * `seq` - event sequence whose leading cut is checked.
///
/// # Returns
///
/// `true` when no unanswered tool call crosses the cut.
///
/// # Panics
///
/// Panics when `seq` is absent from the current surface, a surface sequence has
/// no matching log event, or a tool result has no preceding open call.
#[must_use]
pub fn tool_pairing_balanced_before(session: &Session, seq: u64) -> bool {
    cut_balance(&balance_cache(session), seq, 0)
}

/// Whether the cut immediately after a current surface sequence is tool-pairing balanced.
///
/// # Parameters
///
/// * `session` - session whose surface is checked.
/// * `seq` - event sequence whose trailing cut is checked.
///
/// # Returns
///
/// `true` when no unanswered tool call crosses the cut.
///
/// # Panics
///
/// Panics when `seq` is absent from the current surface, a surface sequence has
/// no matching log event, or a tool result has no preceding open call.
#[must_use]
pub fn tool_pairing_balanced_after(session: &Session, seq: u64) -> bool {
    cut_balance(&balance_cache(session), seq, 1)
}

#[cfg(test)]
mod tests {
    use dsh_session::{
        AssistantMessageData, CallId, ContentBlock, LeftoverEvent, LogEvent, Message, MessageId,
        MessageRole, MessageSource, SESSION_FORMAT_VERSION, Session, SessionEvent, SessionHeader,
        SessionId, SurfaceOp, ToolResultData,
    };

    use crate::{tool_pairing_balanced_after, tool_pairing_balanced_before};

    fn header(id: &str) -> SessionHeader {
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

    fn session_with_unpaired_tool_call() -> Session {
        let mut events = Vec::with_capacity(11);
        for seq in 0..10 {
            events.push(LogEvent::Leftover(LeftoverEvent {
                type_name: "test/pad".into(),
                seq,
                time: seq as i64,
                data: serde_json::json!({}),
                ignorable: Some(true),
                surface_op: None,
                source_event_seqs: None,
            }));
        }
        events.push(LogEvent::Known(SessionEvent::AssistantMessage {
            seq: 10,
            time: 10,
            data: AssistantMessageData {
                turn: 1,
                step: 1,
                message: Message {
                    id: MessageId::new("a"),
                    role: MessageRole::Assistant,
                    content: vec![ContentBlock::ToolCall {
                        id: CallId::new("call-1"),
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
        }));
        Session::from_events(header("unpaired-tool-call"), events)
            .expect("unpaired tool-call session")
    }

    #[test]
    fn pairing_rejects_cut_between_tool_call_and_result() {
        let session = session_with_unpaired_tool_call();
        let call_seq = 10;
        assert!(!tool_pairing_balanced_after(&session, call_seq));
    }

    #[test]
    fn pairing_accepts_the_cut_before_an_unpaired_call() {
        let session = session_with_unpaired_tool_call();
        assert!(tool_pairing_balanced_before(&session, 10));
    }

    fn tool_result(seq: u64) -> SessionEvent {
        SessionEvent::ToolResult {
            seq,
            time: seq as i64,
            data: ToolResultData {
                turn: 1,
                step: 1,
                message: Message {
                    id: MessageId::new("r"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_call_id: CallId::new("orphan"),
                        content: vec![ContentBlock::Text {
                            text: "done".into(),
                        }],
                        is_error: Some(false),
                    }],
                    source: MessageSource::Tool {
                        call_id: CallId::new("orphan"),
                    },
                },
                error: None,
                meta: None,
            },
            surface_op: Some(SurfaceOp::Append),
            source_event_seqs: None,
            ignorable: None,
        }
    }

    #[test]
    #[should_panic(expected = "surface seq 999 not found")]
    fn pairing_rejects_seq_absent_from_surface() {
        let session = Session::new(header("missing-membership"));
        let _ = tool_pairing_balanced_after(&session, 999);
    }

    #[test]
    #[should_panic(expected = "no matching tool-call")]
    fn pairing_rejects_orphan_tool_result() {
        let mut session = Session::new(header("orphan-result"));
        session
            .append(tool_result(0))
            .expect("append orphan result");
        let _ = tool_pairing_balanced_after(&session, 0);
    }
}
