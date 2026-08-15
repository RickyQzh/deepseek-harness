//! Lossless storage packing for `assistant/chunk` delta runs.
//!
//! Consecutive same-block text, reasoning, or tool-call deltas become one
//! `text-chunks` / `reasoning-chunks` / `tool-call-chunks` row. Storage rows
//! are a durable-encoding vocabulary, not session events: they never enter the
//! event log and use slash-less type tags. Anything not fully recognized is
//! stored verbatim.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::{AssistantChunkData, StreamChunk};
use crate::{CallId, LogEvent, SessionError, SessionEvent, decode_log_event};

/// Minimum members before a run packs. Below it a row's envelope rivals the
/// event lines it replaces. A format constant, not a tunable: both layouts
/// decode identically, so changing it never invalidates stored logs.
pub const MIN_CHUNK_RUN: usize = 3;

/// The chunk kinds that may pack; block boundaries, usage, and finish stay one
/// event per line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeltaKind {
    Text,
    Reasoning,
    ToolCall,
}

/// Payload of a `text-chunks` / `reasoning-chunks` row: placement, block index,
/// timestamp gaps, and one text fragment per member. Member `k` reconstructs as
/// seq `seq0 + k` and time `time0` plus the first `k` gaps; a gap may be
/// negative when the wall clock stepped backwards between events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRunData {
    /// Turn number shared by every member.
    pub turn: u64,
    /// Step number shared by every member.
    pub step: u64,
    /// Stream block index every member shares.
    pub index: u32,
    /// Epoch-ms gaps between consecutive members; length is one less than the member count.
    pub dt: Vec<i64>,
    /// One text fragment per member, never joined — token boundaries are data.
    pub texts: Vec<String>,
}

/// Payload of a `tool-call-chunks` row: the run-constant call identity plus
/// each member's raw arguments fragment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallRunData {
    /// Turn number shared by every member.
    pub turn: u64,
    /// Step number shared by every member.
    pub step: u64,
    /// Stream block index every member shares.
    pub index: u32,
    /// Epoch-ms gaps between consecutive members; length is one less than the member count.
    pub dt: Vec<i64>,
    /// Provider-issued call id shared by every member.
    pub id: CallId,
    /// Present iff every member carried it, with one uniform value (a mixed run never packs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// One `argumentsDelta` fragment per member.
    pub args: Vec<String>,
}

/// A packed run of consecutive delta chunk events, discriminated on `type`.
/// `seq0`/`time0` anchor the first member; text and reasoning rows share
/// [`TextRunData`], tool-call rows carry [`ToolCallRunData`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChunkRow {
    /// Packed `text-delta` run.
    #[serde(rename = "text-chunks")]
    Text {
        /// Sequence number of the first member.
        seq0: u64,
        /// Epoch milliseconds of the first member.
        time0: i64,
        /// Shared placement plus per-member texts and gaps.
        data: TextRunData,
    },
    /// Packed `reasoning-delta` run.
    #[serde(rename = "reasoning-chunks")]
    Reasoning {
        /// Sequence number of the first member.
        seq0: u64,
        /// Epoch milliseconds of the first member.
        time0: i64,
        /// Shared placement plus per-member texts and gaps.
        data: TextRunData,
    },
    /// Packed `tool-call-delta` run.
    #[serde(rename = "tool-call-chunks")]
    ToolCall {
        /// Sequence number of the first member.
        seq0: u64,
        /// Epoch milliseconds of the first member.
        time0: i64,
        /// Shared placement, call identity, and per-member argument fragments.
        data: ToolCallRunData,
    },
}

/// One durable log line's JSON value: a session event verbatim, or a packed chunk row.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum StorageRecord {
    /// A session event stored as one JSONL line.
    Event(SessionEvent),
    /// A packed delta run stored as one JSONL line.
    Chunk(ChunkRow),
}

impl ChunkRow {
    fn tag(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text-chunks",
            Self::Reasoning { .. } => "reasoning-chunks",
            Self::ToolCall { .. } => "tool-call-chunks",
        }
    }

    fn seq0(&self) -> u64 {
        match self {
            Self::Text { seq0, .. }
            | Self::Reasoning { seq0, .. }
            | Self::ToolCall { seq0, .. } => *seq0,
        }
    }

    fn time0(&self) -> i64 {
        match self {
            Self::Text { time0, .. }
            | Self::Reasoning { time0, .. }
            | Self::ToolCall { time0, .. } => *time0,
        }
    }

    fn dt(&self) -> &[i64] {
        match self {
            Self::Text { data, .. } | Self::Reasoning { data, .. } => &data.dt,
            Self::ToolCall { data, .. } => &data.dt,
        }
    }

    fn payload_len(&self) -> usize {
        match self {
            Self::Text { data, .. } | Self::Reasoning { data, .. } => data.texts.len(),
            Self::ToolCall { data, .. } => data.args.len(),
        }
    }

    fn payload_key(&self) -> &'static str {
        match self {
            Self::Text { .. } | Self::Reasoning { .. } => "texts",
            Self::ToolCall { .. } => "args",
        }
    }
}

/// Exact-key check: `value` is an object with every key in `keys` and nothing else.
fn has_exact_keys(value: &Value, keys: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
}

/// Classify an event for packing: its delta kind when the entire JSON object
/// (envelope, data, chunk — exact keys) is whitelisted, else `None` (store
/// verbatim). Live typed events already have the exact field set;
/// `ignorable: None` is skip-serialized.
fn classify(event: &SessionEvent) -> Option<DeltaKind> {
    let SessionEvent::AssistantChunk { data, .. } = event else {
        return None;
    };
    let value = serde_json::to_value(event).ok()?;
    if !has_exact_keys(&value, &["type", "seq", "time", "data"]) {
        return None;
    }
    let data_value = value.get("data")?;
    if !has_exact_keys(data_value, &["turn", "step", "chunk"]) {
        return None;
    }
    let chunk_value = data_value.get("chunk")?;
    match &data.chunk {
        StreamChunk::TextDelta { .. } => {
            has_exact_keys(chunk_value, &["type", "index", "text"]).then_some(DeltaKind::Text)
        }
        StreamChunk::ReasoningDelta { .. } => {
            has_exact_keys(chunk_value, &["type", "index", "text"]).then_some(DeltaKind::Reasoning)
        }
        StreamChunk::ToolCallDelta { name, .. } => {
            let keys_ok = if name.is_some() {
                has_exact_keys(
                    chunk_value,
                    &["type", "index", "id", "name", "argumentsDelta"],
                )
            } else {
                has_exact_keys(chunk_value, &["type", "index", "id", "argumentsDelta"])
            };
            keys_ok.then_some(DeltaKind::ToolCall)
        }
        StreamChunk::BlockStart { .. }
        | StreamChunk::BlockEnd { .. }
        | StreamChunk::Usage { .. }
        | StreamChunk::Finish { .. } => None,
    }
}

fn as_chunk(event: &SessionEvent) -> Option<(&AssistantChunkData, u64, i64)> {
    match event {
        SessionEvent::AssistantChunk {
            seq, time, data, ..
        } => Some((data, *seq, *time)),
        _ => None,
    }
}

fn chunk_index(chunk: &StreamChunk) -> Option<u32> {
    match chunk {
        StreamChunk::TextDelta { index, .. }
        | StreamChunk::ReasoningDelta { index, .. }
        | StreamChunk::ToolCallDelta { index, .. } => Some(*index),
        StreamChunk::BlockStart { .. }
        | StreamChunk::BlockEnd { .. }
        | StreamChunk::Usage { .. }
        | StreamChunk::Finish { .. } => None,
    }
}

/// Whether `next` extends a run ending in `prev` (same kind already checked by the caller).
fn continues(prev: &SessionEvent, next: &SessionEvent, kind: DeltaKind) -> bool {
    let Some((prev_data, prev_seq, prev_time)) = as_chunk(prev) else {
        return false;
    };
    let Some((next_data, next_seq, next_time)) = as_chunk(next) else {
        return false;
    };
    if prev_seq.checked_add(1) != Some(next_seq) {
        return false;
    }
    if next_time.checked_sub(prev_time).is_none() {
        return false;
    }
    if next_data.turn != prev_data.turn || next_data.step != prev_data.step {
        return false;
    }
    if chunk_index(&next_data.chunk) != chunk_index(&prev_data.chunk) {
        return false;
    }
    if kind != DeltaKind::ToolCall {
        return true;
    }
    match (&prev_data.chunk, &next_data.chunk) {
        (
            StreamChunk::ToolCallDelta {
                id: prev_id,
                name: prev_name,
                ..
            },
            StreamChunk::ToolCallDelta {
                id: next_id,
                name: next_name,
                ..
            },
        ) => {
            prev_id == next_id
                && prev_name.is_some() == next_name.is_some()
                && prev_name == next_name
        }
        _ => false,
    }
}

fn run_texts(run: &[SessionEvent]) -> Vec<String> {
    run.iter()
        .map(|event| match event {
            SessionEvent::AssistantChunk {
                data:
                    AssistantChunkData {
                        chunk:
                            StreamChunk::TextDelta { text, .. }
                            | StreamChunk::ReasoningDelta { text, .. },
                        ..
                    },
                ..
            } => text.clone(),
            _ => unreachable!("text/reasoning run members are classified deltas"),
        })
        .collect()
}

fn run_args(run: &[SessionEvent]) -> Vec<String> {
    run.iter()
        .map(|event| match event {
            SessionEvent::AssistantChunk {
                data:
                    AssistantChunkData {
                        chunk:
                            StreamChunk::ToolCallDelta {
                                arguments_delta, ..
                            },
                        ..
                    },
                ..
            } => arguments_delta.clone(),
            _ => unreachable!("tool-call run members are classified deltas"),
        })
        .collect()
}

/// Build the row for a completed run (`run.len() >= MIN_CHUNK_RUN`, uniform per [`continues`]).
fn build_row(kind: DeltaKind, run: &[SessionEvent]) -> ChunkRow {
    let first = run
        .first()
        .and_then(as_chunk)
        .expect("a packed run has at least one classified member");
    let (first_data, seq0, time0) = first;
    let index = chunk_index(&first_data.chunk).expect("classified deltas carry a block index");
    let dt: Vec<i64> = run
        .windows(2)
        .map(|pair| {
            let prev_time = as_chunk(&pair[0]).expect("run members are classified").2;
            let next_time = as_chunk(&pair[1]).expect("run members are classified").2;
            next_time
                .checked_sub(prev_time)
                .expect("continues already accepted this gap")
        })
        .collect();
    match kind {
        DeltaKind::ToolCall => {
            let (id, name) = match &first_data.chunk {
                StreamChunk::ToolCallDelta { id, name, .. } => (id.clone(), name.clone()),
                _ => unreachable!("tool-call kind has a tool-call-delta first member"),
            };
            ChunkRow::ToolCall {
                seq0,
                time0,
                data: ToolCallRunData {
                    turn: first_data.turn,
                    step: first_data.step,
                    index,
                    dt,
                    id,
                    name,
                    args: run_args(run),
                },
            }
        }
        DeltaKind::Text | DeltaKind::Reasoning => {
            let data = TextRunData {
                turn: first_data.turn,
                step: first_data.step,
                index,
                dt,
                texts: run_texts(run),
            };
            if kind == DeltaKind::Text {
                ChunkRow::Text { seq0, time0, data }
            } else {
                ChunkRow::Reasoning { seq0, time0, data }
            }
        }
    }
}

fn flush_run(out: &mut Vec<StorageRecord>, kind: Option<DeltaKind>, run: &[SessionEvent]) {
    match kind {
        Some(kind) if run.len() >= MIN_CHUNK_RUN => {
            out.push(StorageRecord::Chunk(build_row(kind, run)));
        }
        _ => out.extend(run.iter().cloned().map(StorageRecord::Event)),
    }
}

/// Pack an event batch for storage: each run of at least [`MIN_CHUNK_RUN`]
/// consecutive whitelisted same-kind, same-block delta chunk events becomes one
/// [`ChunkRow`]; every other event passes through verbatim, in order.
///
/// Pure and stateless — safe over any slice, including a batch whose runs were
/// split by flush boundaries (the split runs simply pack per batch).
#[must_use]
pub fn pack_chunk_runs(events: &[SessionEvent]) -> Vec<StorageRecord> {
    let mut out = Vec::new();
    let mut kind: Option<DeltaKind> = None;
    let mut run_start = 0usize;
    let mut run_end = 0usize;
    for (index, event) in events.iter().enumerate() {
        let Some(next_kind) = classify(event) else {
            flush_run(&mut out, kind, &events[run_start..run_end]);
            kind = None;
            run_start = index;
            run_end = index;
            out.push(StorageRecord::Event(event.clone()));
            continue;
        };
        let last = run_end
            .checked_sub(1)
            .filter(|&last| last >= run_start)
            .and_then(|last| events.get(last));
        if Some(next_kind) == kind && last.is_some_and(|prev| continues(prev, event, next_kind)) {
            run_end = index + 1;
            continue;
        }
        flush_run(&mut out, kind, &events[run_start..run_end]);
        kind = Some(next_kind);
        run_start = index;
        run_end = index + 1;
    }
    flush_run(&mut out, kind, &events[run_start..run_end]);
    out
}

fn malformed(tag: &str, why: impl std::fmt::Display) -> SessionError {
    SessionError::Corrupt(format!("malformed {tag} storage row: {why}"))
}

/// Validate payload arity and that reconstructed member seq/time stay in `i64`.
fn validate_row(row: &ChunkRow) -> Result<(), SessionError> {
    let tag = row.tag();
    let payload_len = row.payload_len();
    if payload_len == 0 {
        return Err(malformed(
            tag,
            format!("{} must be a non-empty string array", row.payload_key()),
        ));
    }
    let dt = row.dt();
    if dt.len() != payload_len - 1 {
        return Err(malformed(
            tag,
            format!(
                "dt length {} does not match {payload_len} members",
                dt.len()
            ),
        ));
    }
    let last_offset = i64::try_from(payload_len - 1)
        .map_err(|_| malformed(tag, "member seqs must stay in i64 range"))?;
    let seq0 = i64::try_from(row.seq0())
        .map_err(|_| malformed(tag, "member seqs must stay in i64 range"))?;
    if seq0.checked_add(last_offset).is_none() {
        return Err(malformed(tag, "member seqs must stay in i64 range"));
    }
    let mut time = row.time0();
    for gap in dt {
        time = time
            .checked_add(*gap)
            .ok_or_else(|| malformed(tag, "member times must stay in i64 range"))?;
    }
    Ok(())
}

/// Expand a validated row back into its exact original events, in order.
fn expand_row(row: &ChunkRow) -> Vec<SessionEvent> {
    let mut time = row.time0();
    match row {
        ChunkRow::Text { seq0, data, .. } => data
            .texts
            .iter()
            .enumerate()
            .map(|(k, text)| {
                if k > 0 {
                    time += data.dt[k - 1];
                }
                SessionEvent::AssistantChunk {
                    seq: seq0 + k as u64,
                    time,
                    data: AssistantChunkData {
                        turn: data.turn,
                        step: data.step,
                        chunk: StreamChunk::TextDelta {
                            index: data.index,
                            text: text.clone(),
                        },
                    },
                    ignorable: None,
                }
            })
            .collect(),
        ChunkRow::Reasoning { seq0, data, .. } => data
            .texts
            .iter()
            .enumerate()
            .map(|(k, text)| {
                if k > 0 {
                    time += data.dt[k - 1];
                }
                SessionEvent::AssistantChunk {
                    seq: seq0 + k as u64,
                    time,
                    data: AssistantChunkData {
                        turn: data.turn,
                        step: data.step,
                        chunk: StreamChunk::ReasoningDelta {
                            index: data.index,
                            text: text.clone(),
                        },
                    },
                    ignorable: None,
                }
            })
            .collect(),
        ChunkRow::ToolCall { seq0, data, .. } => data
            .args
            .iter()
            .enumerate()
            .map(|(k, arguments_delta)| {
                if k > 0 {
                    time += data.dt[k - 1];
                }
                SessionEvent::AssistantChunk {
                    seq: seq0 + k as u64,
                    time,
                    data: AssistantChunkData {
                        turn: data.turn,
                        step: data.step,
                        chunk: StreamChunk::ToolCallDelta {
                            index: data.index,
                            id: data.id.clone(),
                            name: data.name.clone(),
                            arguments_delta: arguments_delta.clone(),
                        },
                    },
                    ignorable: None,
                }
            })
            .collect(),
    }
}

/// Decode one parsed JSON object into the session event(s) it stores.
///
/// The three packed-row tags validate and expand. Any other value is decoded as
/// one first-party event. A leftover is not a chunk row.
///
/// # Errors
///
/// [`SessionError::Corrupt`] when a row-tagged value is malformed, reconstructed
/// member seq/time leave `i64`, or the value is an unknown leftover.
/// Propagates [`decode_log_event`] errors for non-row values.
pub fn decode_storage_record(value: Value) -> Result<Vec<SessionEvent>, SessionError> {
    let tag = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if matches!(
        tag.as_str(),
        "text-chunks" | "reasoning-chunks" | "tool-call-chunks"
    ) {
        let row: ChunkRow = serde_json::from_value(value).map_err(|error| {
            SessionError::Corrupt(format!("malformed {tag} storage row: {error}"))
        })?;
        validate_row(&row)?;
        return Ok(expand_row(&row));
    }
    match decode_log_event(value)? {
        LogEvent::Known(event) => Ok(vec![event]),
        LogEvent::Leftover(_) => Err(SessionError::Corrupt(
            "chunk decoder received an unknown leftover".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{ChunkRow, StorageRecord, decode_storage_record, pack_chunk_runs};
    use crate::SessionEvent;
    use crate::message::{AssistantChunkData, StreamChunk};
    use serde_json::json;

    fn text_delta(seq: u64, time: i64, text: &str) -> SessionEvent {
        SessionEvent::AssistantChunk {
            seq,
            time,
            data: AssistantChunkData {
                turn: 1,
                step: 1,
                chunk: StreamChunk::TextDelta {
                    index: 0,
                    text: text.into(),
                },
            },
            ignorable: None,
        }
    }

    #[test]
    fn packs_five_text_deltas_and_round_trips() {
        let events: Vec<_> = (0..5)
            .map(|k| text_delta(k, 1000 + 10 * k as i64, &format!("t{k}")))
            .collect();
        let packed = pack_chunk_runs(&events);
        assert_eq!(packed.len(), 1);
        match &packed[0] {
            StorageRecord::Chunk(ChunkRow::Text { seq0, time0, data }) => {
                assert_eq!(*seq0, 0);
                assert_eq!(*time0, 1000);
                assert_eq!(data.dt, vec![10, 10, 10, 10]);
                assert_eq!(data.texts, ["t0", "t1", "t2", "t3", "t4"]);
            }
            _ => panic!("expected text-chunks"),
        }
        let value = serde_json::to_value(&packed[0]).expect("row json");
        let decoded = decode_storage_record(value).expect("expand");
        assert_eq!(decoded, events);
    }

    #[test]
    fn leaves_runs_shorter_than_three_verbatim() {
        let events = vec![text_delta(0, 1000, "a"), text_delta(1, 1010, "b")];
        let packed = pack_chunk_runs(&events);
        assert_eq!(packed.len(), 2);
        assert!(matches!(packed[0], StorageRecord::Event(_)));
    }

    #[test]
    fn malformed_text_chunks_row_fails_loud() {
        let error = decode_storage_record(json!({
            "type": "text-chunks",
            "seq0": 0,
            "time0": 1,
            "data": {"turn": 1, "step": 1, "index": 0, "dt": [], "texts": []}
        }))
        .expect_err("empty texts");
        assert!(error.to_string().contains("malformed text-chunks"));
    }

    #[test]
    fn non_row_passes_through_as_one_event() {
        let value = json!({"type":"turn/start","seq":0,"time":1,"data":{"turn":1}});
        let decoded = decode_storage_record(value).expect("event");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].event_type(), "turn/start");
    }
}
