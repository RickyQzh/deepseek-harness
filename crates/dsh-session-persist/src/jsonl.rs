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

#[cfg(test)]
mod phase2_exit_tests {
    use super::{decode_session_log, encode_session_log};
    use crate::zstd::{compress_zstd_frame, decompress_zstd_frames};
    use dsh_session::{
        LogEvent, SESSION_FORMAT_VERSION, Session, SessionEvent, decode_log_event,
        fold_request_header, interrupted_turn_closers,
    };
    use std::fs;
    use std::path::PathBuf;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../examples/headless-agent/tests/snapshots/headless-profile/session.expected.jsonl",
        )
    }

    fn parse_jsonl(text: &str) -> Vec<serde_json::Value> {
        text.lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).expect("json"))
            .collect()
    }

    fn known_events(events: &[LogEvent]) -> Vec<SessionEvent> {
        events
            .iter()
            .filter_map(|event| match event {
                LogEvent::Known(event) => Some(event.clone()),
                LogEvent::Leftover(_) => None,
            })
            .collect()
    }

    #[test]
    fn headless_profile_fixture_round_trips_normalized() {
        let original = fs::read_to_string(fixture_path()).expect("read fixture");
        let (header, events) = decode_session_log(&original).expect("decode fixture");
        assert_eq!(header.version, SESSION_FORMAT_VERSION);
        assert_eq!(events.len(), 32);
        assert!(
            events
                .iter()
                .all(|event| matches!(event, LogEvent::Known(_)))
        );
        let encoded = encode_session_log(&header, &events, false).expect("encode");
        assert_eq!(parse_jsonl(&original), parse_jsonl(&encoded));
    }

    #[test]
    fn headless_profile_fixture_survives_pack_chunks_true() {
        let original = fs::read_to_string(fixture_path()).expect("read fixture");
        let (header, events) = decode_session_log(&original).expect("decode");
        let packed = encode_session_log(&header, &events, true).expect("pack");
        let (header2, events2) = decode_session_log(&packed).expect("decode packed");
        assert_eq!(header, header2);
        assert_eq!(events, events2);
    }

    #[test]
    fn headless_profile_fixture_survives_one_zstd_frame() {
        let original = fs::read_to_string(fixture_path()).expect("read fixture");
        let encoded = compress_zstd_frame(original.as_bytes()).expect("zstd");
        let plain = decompress_zstd_frames(&encoded).expect("unzstd");
        assert_eq!(plain, original.as_bytes());
        decode_session_log(std::str::from_utf8(&plain).expect("utf8")).expect("decode");
    }

    #[test]
    fn headless_profile_fixture_derives_four_surface_messages() {
        let original = fs::read_to_string(fixture_path()).expect("read fixture");
        let (header, events) = decode_session_log(&original).expect("decode");
        let session = Session::from_events(header, events.clone()).expect("session");
        let messages = session.derive_messages();
        assert_eq!(messages.len(), 5);
        assert!(
            matches!(&messages[0].content[0], dsh_session::ContentBlock::Text { text } if text.contains("Prove the product headless"))
        );
        assert!(matches!(
            &messages[3].content[0],
            dsh_session::ContentBlock::ToolResult { .. }
        ));
        assert!(
            matches!(&messages[4].content[0], dsh_session::ContentBlock::Text { text } if text.contains("CLI tool round trip complete"))
        );
        let known = known_events(&events);
        assert!(fold_request_header(&known, None).is_some());
        assert!(interrupted_turn_closers(&known).is_empty());
    }

    #[test]
    fn refuses_newer_format_version_with_upgrade_direction() {
        let text = concat!(
            r#"{"type":"session","version":99,"id":"workspace-context-resume","createdAt":1,"delegationDepth":0}"#,
            "\n",
            r#"{"type":"turn/start","seq":0,"time":1,"data":{"turn":1}}"#,
            "\n",
        );
        let error = decode_session_log(text).expect_err("refuse");
        assert_eq!(
            error.to_string(),
            "session \"workspace-context-resume\" uses log format v99, but this harness reads only v0: the log was written by a newer harness — upgrade the harness to open it"
        );
    }

    #[test]
    fn refuses_older_format_version_without_upgrade_path() {
        let text = concat!(
            r#"{"type":"session","version":-1,"id":"v-older","createdAt":1,"delegationDepth":0}"#,
            "\n",
        );
        let error = decode_session_log(text).expect_err("refuse");
        assert_eq!(
            error.to_string(),
            "session \"v-older\" uses log format v-1, older than the supported v0, and this build ships no upgrade path for it"
        );
    }

    #[test]
    fn refuses_unknown_required_event_type() {
        let text = concat!(
            r#"{"type":"session","version":0,"id":"workspace-context-resume","createdAt":1,"delegationDepth":0}"#,
            "\n",
            r#"{"type":"turn/start","seq":0,"time":1,"data":{"turn":1}}"#,
            "\n",
            r#"{"type":"turn/end","seq":1,"time":2,"data":{"turn":1,"reason":{"kind":"completed"}}}"#,
            "\n",
            r#"{"type":"future/event","seq":2,"time":3,"data":{"payload":1}}"#,
            "\n",
        );
        let error = decode_session_log(text).expect_err("refuse");
        assert_eq!(
            error.to_string(),
            "session \"workspace-context-resume\" contains event type \"future/event\" (seq 2) unknown to this harness and not marked ignorable; refusing to interpret the log — it was likely written by a newer harness"
        );
    }

    #[test]
    fn keeps_unknown_ignorable_leftover() {
        let text = concat!(
            r#"{"type":"session","version":0,"id":"unknown-ignorable","createdAt":1,"delegationDepth":0}"#,
            "\n",
            r#"{"type":"turn/start","seq":0,"time":1,"data":{"turn":1}}"#,
            "\n",
            r#"{"type":"turn/end","seq":1,"time":2,"data":{"turn":1,"reason":{"kind":"completed"}}}"#,
            "\n",
            r#"{"type":"future/event","seq":2,"time":99,"data":{"payload":1},"ignorable":true}"#,
            "\n",
        );
        let (_header, events) = decode_session_log(text).expect("load");
        assert!(
            events
                .iter()
                .any(|event| event.event_type() == "future/event")
        );
        match decode_log_event(serde_json::json!({
            "type": "future/event",
            "seq": 2,
            "time": 99,
            "data": {"payload": 1},
            "ignorable": true
        }))
        .expect("leftover")
        {
            LogEvent::Leftover(leftover) => assert_eq!(leftover.type_name, "future/event"),
            LogEvent::Known(_) => panic!("expected leftover"),
        }
    }
}
