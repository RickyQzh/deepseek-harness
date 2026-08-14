//! First-party session event type strings.
//!
//! This list is the Rust owner of the same vocabulary as
//! `packages/core/session/src/known-event-types.ts`. It is not generated from
//! composition and must not grow via typetag or inventory.

/// Every `SessionEventMap` member declared in this repository.
pub const KNOWN_SESSION_EVENT_TYPES: &[&str] = &[
    "agent-preset/selected",
    "agent/inbox/spliced",
    "approval/asked",
    "approval/decided",
    "approval/policy",
    "assistant/chunk",
    "assistant/message",
    "command/done",
    "command/run",
    "compaction/end",
    "compaction/prune",
    "compaction/start",
    "compaction/summary",
    "feedback/record",
    "goal/change",
    "hook/invoked",
    "hook/result",
    "llm/retry",
    "llm/retry-started",
    "permission/preset",
    "plan/mode",
    "request/context",
    "request/header",
    "sandbox/mode",
    "schedule/change",
    "session/end-seed",
    "session/title",
    "session/title-llm-request",
    "step/end",
    "step/start",
    "subagent/descriptor",
    "todo/write",
    "tool-workflow/agent-end",
    "tool-workflow/agent-start",
    "tool-workflow/run-end",
    "tool-workflow/run-start",
    "tool/call",
    "tool/code-dispatch",
    "tool/code-dispatch-start",
    "tool/result",
    "turn/end",
    "turn/start",
    "user/message",
    "web/deepseek-search-llm-request",
];

/// Whether `type_name` is a first-party session event type.
#[must_use]
pub fn is_known_session_event_type(type_name: &str) -> bool {
    KNOWN_SESSION_EVENT_TYPES.contains(&type_name)
}

#[cfg(test)]
mod tests {
    use super::{KNOWN_SESSION_EVENT_TYPES, is_known_session_event_type};
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    fn ts_catalog_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/core/session/src/known-event-types.ts")
    }

    fn parse_ts_known_types(source: &str) -> Vec<String> {
        let start = source
            .find("new Set([")
            .expect("KNOWN_SESSION_EVENT_TYPES Set");
        let rest = &source[start..];
        let end = rest.find(']').expect("closing Set bracket");
        rest[..end]
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                let start = trimmed.find('\'')?;
                let end = trimmed.rfind('\'')?;
                if end <= start {
                    return None;
                }
                Some(trimmed[start + 1..end].to_string())
            })
            .collect()
    }

    #[test]
    fn rust_catalog_matches_typescript_known_event_types() {
        let source = fs::read_to_string(ts_catalog_path()).expect("read known-event-types.ts");
        let ts = parse_ts_known_types(&source);
        assert_eq!(
            ts.as_slice(),
            KNOWN_SESSION_EVENT_TYPES,
            "Rust KNOWN_SESSION_EVENT_TYPES must list the same types in the same order as known-event-types.ts"
        );
    }

    #[test]
    fn rust_catalog_is_unique_and_has_forty_four_types() {
        let set: BTreeSet<_> = KNOWN_SESSION_EVENT_TYPES.iter().copied().collect();
        assert_eq!(set.len(), KNOWN_SESSION_EVENT_TYPES.len());
        assert_eq!(KNOWN_SESSION_EVENT_TYPES.len(), 44);
        assert!(is_known_session_event_type("user/message"));
        assert!(!is_known_session_event_type("future/event"));
    }
}
