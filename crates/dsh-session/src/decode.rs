//! Read-side format guards.

use serde_json::Value;

use crate::error::{SessionError, SessionFormatError};
use crate::{
    LeftoverEvent, LogEvent, SESSION_FORMAT_VERSION, SessionEvent, is_known_session_event_type,
};

pub use crate::error::{session_format_version_refusal, unknown_required_event_refusal};

/// Refuse a header carrying a format version this build does not read, before shape validation.
///
/// # Errors
///
/// [`SessionFormatError::Unsupported`] when `version` is present and not [`SESSION_FORMAT_VERSION`].
pub fn refuse_foreign_format_version(parsed: &Value) -> Result<(), SessionFormatError> {
    let Some(object) = parsed.as_object() else {
        return Ok(());
    };
    let Some(version) = object.get("version").and_then(Value::as_i64) else {
        return Ok(());
    };
    if version == i64::from(SESSION_FORMAT_VERSION) {
        return Ok(());
    }
    let id = match object.get("id") {
        Some(Value::String(id)) => id.clone(),
        Some(other) => other.to_string(),
        None => "unknown".into(),
    };
    Err(SessionFormatError::Unsupported(
        session_format_version_refusal(&id, version),
    ))
}

/// Decode one JSON object into a known event or an ignorable leftover.
///
/// # Errors
///
/// [`SessionFormatError::Unsupported`] when `type` is unknown and `ignorable` is not `true`.
pub fn decode_log_event(value: Value) -> Result<LogEvent, SessionError> {
    decode_log_event_for_session("session-unknown", value)
}

/// [`decode_log_event`] naming `session_id` in refusal text.
///
/// # Errors
///
/// Same as [`decode_log_event`].
pub fn decode_log_event_for_session(
    session_id: &str,
    value: Value,
) -> Result<LogEvent, SessionError> {
    let type_name = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if is_known_session_event_type(&type_name) {
        let event: SessionEvent = serde_json::from_value(value).map_err(|error| {
            SessionError::Corrupt(format!("corrupt session event {type_name}: {error}"))
        })?;
        return Ok(LogEvent::Known(event));
    }
    let seq = value.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let ignorable = value.get("ignorable").and_then(Value::as_bool) == Some(true);
    if !ignorable {
        return Err(
            SessionFormatError::Unsupported(unknown_required_event_refusal(
                session_id, &type_name, seq,
            ))
            .into(),
        );
    }
    let leftover: LeftoverEvent = serde_json::from_value(value).map_err(|error| {
        SessionError::Corrupt(format!("corrupt leftover event {type_name}: {error}"))
    })?;
    Ok(LogEvent::Leftover(leftover))
}

#[cfg(test)]
mod tests {
    use super::{
        decode_log_event, refuse_foreign_format_version, session_format_version_refusal,
        unknown_required_event_refusal,
    };
    use crate::LogEvent;
    use serde_json::json;

    #[test]
    fn newer_version_names_upgrade_direction() {
        assert_eq!(
            session_format_version_refusal("workspace-context-resume", 99),
            "session \"workspace-context-resume\" uses log format v99, but this harness reads only v0: the log was written by a newer harness — upgrade the harness to open it"
        );
    }

    #[test]
    fn older_version_names_missing_upgrade_path() {
        assert_eq!(
            session_format_version_refusal("v-older", -1),
            "session \"v-older\" uses log format v-1, older than the supported v0, and this build ships no upgrade path for it"
        );
    }

    #[test]
    fn refuse_foreign_version_before_shape() {
        let parsed = json!({"type":"session","version":42,"id":"future-shape","futureOnly":true});
        let error = refuse_foreign_format_version(&parsed).expect_err("must refuse");
        assert!(error.to_string().contains("written by a newer harness"));
        assert!(error.to_string().contains("session \"future-shape\""));
    }

    #[test]
    fn refuse_foreign_version_stringifies_non_string_id() {
        let parsed = json!({"type":"session","version":42,"id":123});
        let error = refuse_foreign_format_version(&parsed).expect_err("must refuse");
        assert!(
            error
                .to_string()
                .contains("session \"123\" uses log format v42")
        );
    }

    #[test]
    fn non_object_header_is_not_a_format_refusal() {
        assert!(refuse_foreign_format_version(&json!(42)).is_ok());
    }

    #[test]
    fn same_version_is_not_a_format_refusal() {
        assert!(
            refuse_foreign_format_version(&json!({"type":"session","version":0,"id":"s"})).is_ok()
        );
    }

    #[test]
    fn unknown_required_event_refuses() {
        let error = decode_log_event(json!({
            "type": "future/event",
            "seq": 2,
            "time": 3,
            "data": {"payload": 1}
        }))
        .expect_err("must refuse");
        assert_eq!(
            error.to_string(),
            unknown_required_event_refusal("session-unknown", "future/event", 2)
        );
        assert!(error.to_string().contains("not marked ignorable"));
    }

    #[test]
    fn unknown_ignorable_event_is_kept_as_leftover() {
        let event = decode_log_event(json!({
            "type": "future/event",
            "seq": 2,
            "time": 3,
            "data": {"payload": 1},
            "ignorable": true
        }))
        .expect("leftover");
        match event {
            LogEvent::Leftover(leftover) => {
                assert_eq!(leftover.type_name, "future/event");
                assert_eq!(leftover.seq, 2);
                assert_eq!(leftover.ignorable, Some(true));
            }
            LogEvent::Known(_) => panic!("expected leftover"),
        }
    }

    #[test]
    fn known_event_decodes_as_known() {
        let event = decode_log_event(json!({
            "type": "turn/start",
            "seq": 0,
            "time": 1,
            "data": {"turn": 1}
        }))
        .expect("known");
        assert!(matches!(event, LogEvent::Known(_)));
    }

    #[test]
    fn ignorable_false_is_required() {
        let error = decode_log_event(json!({
            "type": "future/event",
            "seq": 2,
            "time": 3,
            "data": {},
            "ignorable": false
        }))
        .expect_err("false is not the marker");
        assert!(error.to_string().contains("not marked ignorable"));
    }
}
