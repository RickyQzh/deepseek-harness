//! JSONL encode and decode for session logs.

use serde_json::Value;

use crate::PersistError;
use crate::header_line::{parse_header_record, to_header_line};
use dsh_session::{
    LogEvent, SessionHeader, decode_log_event_for_session, decode_storage_record, pack_chunk_runs,
};

fn json_line<T: serde::Serialize>(value: &T) -> Result<String, PersistError> {
    serde_json::to_string(value).map_err(|error| {
        PersistError::Corrupt(format!("failed to serialize session log line: {error}"))
    })
}

/// Encode a header and events as JSONL, ending with a trailing newline.
///
/// When `pack_chunks` is true, consecutive [`LogEvent::Known`] runs are packed
/// with [`pack_chunk_runs`]. [`LogEvent::Leftover`] rows serialize as themselves
/// and are never packed.
///
/// # Errors
///
/// [`PersistError::Corrupt`] when a header or event line fails to serialize.
pub fn encode_session_log(
    header: &SessionHeader,
    events: &[LogEvent],
    pack_chunks: bool,
) -> Result<String, PersistError> {
    let mut lines = Vec::with_capacity(events.len() + 1);
    lines.push(json_line(&to_header_line(header))?);
    let mut index = 0;
    while index < events.len() {
        match &events[index] {
            LogEvent::Leftover(leftover) => {
                lines.push(json_line(leftover)?);
                index += 1;
            }
            LogEvent::Known(_) => {
                let mut known = Vec::new();
                while let Some(LogEvent::Known(event)) = events.get(index) {
                    known.push(event.clone());
                    index += 1;
                }
                if pack_chunks {
                    for record in pack_chunk_runs(&known) {
                        lines.push(json_line(&record)?);
                    }
                } else {
                    for event in &known {
                        lines.push(json_line(event)?);
                    }
                }
            }
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    Ok(text)
}

/// Decode a JSONL session log into its header and event rows.
///
/// Splits on `\n` and ignores a final empty line. Packed chunk-row tags expand
/// through [`decode_storage_record`]; every other line uses
/// [`decode_log_event_for_session`].
///
/// # Errors
///
/// [`PersistError::Format`] when the header version is foreign or a required
/// unknown event is not ignorable.
/// [`PersistError::Session`] when a packed row or known event is malformed.
/// [`PersistError::Corrupt`] when a line is not valid JSON or the first line
/// is not a session header.
pub fn decode_session_log(text: &str) -> Result<(SessionHeader, Vec<LogEvent>), PersistError> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let mut lines = lines.into_iter();
    let header_line = lines.next().ok_or_else(|| {
        PersistError::Corrupt("corrupt session log: first line is not a session header".into())
    })?;
    let header = parse_header_record(header_line)?;
    let mut events = Vec::new();
    for line in lines {
        let value: Value = serde_json::from_str(line).map_err(|_| {
            PersistError::Corrupt("corrupt session log: event line is not valid JSON".into())
        })?;
        let type_name = value.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(
            type_name,
            "text-chunks" | "reasoning-chunks" | "tool-call-chunks"
        ) {
            for event in decode_storage_record(value)? {
                events.push(LogEvent::Known(event));
            }
        } else {
            events.push(decode_log_event_for_session(header.id.as_str(), value)?);
        }
    }
    Ok((header, events))
}

#[cfg(test)]
mod tests {
    use super::{decode_session_log, encode_session_log};
    use dsh_session::{
        LogEvent, SESSION_FORMAT_VERSION, SessionEvent, SessionHeader, SessionId, TurnStartData,
    };

    fn header() -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("s1"),
            created_at: 1,
            cwd: Some("/work".into()),
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: Some(0),
            agent_preset: None,
        }
    }

    #[test]
    fn plaintext_round_trip_one_turn_start() {
        let events = vec![LogEvent::Known(SessionEvent::TurnStart {
            seq: 0,
            time: 1,
            data: TurnStartData { turn: 1 },
            ignorable: None,
        })];
        let text = encode_session_log(&header(), &events, false).expect("encode");
        assert!(text.starts_with("{\"type\":\"session\""));
        assert!(text.contains("\"delegationDepth\":0"));
        let (meta, decoded) = decode_session_log(&text).expect("decode");
        assert_eq!(meta.id.as_str(), "s1");
        assert_eq!(decoded, events);
    }

    #[test]
    fn newer_header_is_unsupported_not_corrupt() {
        let text = "{\"type\":\"session\",\"version\":42,\"id\":\"future-shape\",\"futureOnly\":true}\n{\"future\":\"row\"}\n";
        let error = decode_session_log(text).expect_err("refuse");
        let message = error.to_string();
        assert!(message.contains("written by a newer harness"));
        assert!(message.contains("session \"future-shape\""));
        assert!(!message.contains("first line is not a session header"));
    }

    #[test]
    fn scalar_header_is_corrupt_not_unsupported() {
        let error = decode_session_log("42\n").expect_err("corrupt");
        assert!(
            error
                .to_string()
                .contains("first line is not a session header")
        );
        assert!(!error.to_string().contains("written by a newer harness"));
    }
}
