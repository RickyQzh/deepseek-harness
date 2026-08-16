//! Versioned `subagent/descriptor` payload helpers.
//!
//! The session event variant already exists as [`dsh_session::SessionEvent::SubagentDescriptor`]
//! with `data: Value`. This module owns the version 2 JSON object.

use dsh_session::{LogEvent, SessionEvent};
use serde_json::{Map, Value, json};

/// Current descriptor format version stamped into every appended payload.
pub const SUBAGENT_DESCRIPTOR_VERSION: u32 = 2;

/// Build a one-shot version-2 descriptor object. Omits `label` when it is `None`.
#[must_use]
pub fn snapshot_one_shot_descriptor(provider: &str, label: Option<&str>) -> Value {
    let mut fields = Map::new();
    fields.insert("version".into(), json!(SUBAGENT_DESCRIPTOR_VERSION));
    fields.insert("mode".into(), json!("one-shot"));
    fields.insert("provider".into(), json!(provider));
    if let Some(label) = label {
        fields.insert("label".into(), json!(label));
    }
    Value::Object(fields)
}

/// Return the first `subagent/descriptor` payload in `events`, if any.
#[must_use]
pub fn fold_subagent_descriptor(events: &[LogEvent]) -> Option<Value> {
    for event in events {
        if let LogEvent::Known(SessionEvent::SubagentDescriptor { data, .. }) = event {
            return Some(data.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        SUBAGENT_DESCRIPTOR_VERSION, fold_subagent_descriptor, snapshot_one_shot_descriptor,
    };
    use dsh_session::{LogEvent, SessionEvent};
    use serde_json::json;

    #[test]
    fn one_shot_snapshot_omits_absent_label() {
        let value = snapshot_one_shot_descriptor("spawn", None);
        assert_eq!(
            value,
            json!({
                "version": SUBAGENT_DESCRIPTOR_VERSION,
                "mode": "one-shot",
                "provider": "spawn",
            })
        );
        assert!(value.get("label").is_none());
    }

    #[test]
    fn fold_reads_the_first_descriptor() {
        let first = snapshot_one_shot_descriptor("spawn", Some("a"));
        let events = vec![LogEvent::Known(SessionEvent::SubagentDescriptor {
            seq: 0,
            time: 0,
            data: first.clone(),
            ignorable: None,
        })];
        assert_eq!(fold_subagent_descriptor(&events), Some(first));
        assert_eq!(fold_subagent_descriptor(&[]), None);
    }
}
