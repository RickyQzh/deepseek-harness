//! Out-of-log session storage metadata.

use serde::{Deserialize, Serialize};

use crate::SessionId;

/// On-disk session format version stamped into every newly written header.
///
/// Pinned at `0` for the rewrite: no compatibility promise, no upgrader chain.
pub const SESSION_FORMAT_VERSION: u32 = 0;

/// Coarse product classification for a session created as a subagent child.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionOrigin {
    /// Presentation metadata for a subagent-created session.
    Subagent,
}

/// Immutable validated storage metadata, kept outside the conversation event log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHeader {
    /// On-disk format version. Writers stamp [`SESSION_FORMAT_VERSION`].
    pub version: u32,
    /// Session identity (mirrors the live session id).
    pub id: SessionId,
    /// Non-negative Unix epoch milliseconds when the session was created.
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    /// Absolute working directory the session was created in, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Parent session id when this session was forked.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "parentSession"
    )]
    pub parent_session: Option<SessionId>,
    /// How many leading events were inherited through a seed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "seedLength"
    )]
    pub seed_length: Option<u64>,
    /// Presentation origin for a subagent child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<SessionOrigin>,
    /// Delegation depth; absent means zero at the product layer.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "delegationDepth"
    )]
    pub delegation_depth: Option<u64>,
    /// Agent preset id this session was composed from, when recorded.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "agentPreset"
    )]
    pub agent_preset: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{SESSION_FORMAT_VERSION, SessionHeader, SessionOrigin};
    use crate::SessionId;
    use serde_json::json;

    #[test]
    fn format_version_is_zero() {
        assert_eq!(SESSION_FORMAT_VERSION, 0);
    }

    #[test]
    fn header_round_trips_optional_fields_and_omits_absences() {
        let header = SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("s1"),
            created_at: 1,
            cwd: Some("/work".into()),
            parent_session: Some(SessionId::new("parent")),
            seed_length: Some(3),
            origin: Some(SessionOrigin::Subagent),
            delegation_depth: Some(1),
            agent_preset: Some("standard".into()),
        };
        let value = serde_json::to_value(&header).expect("serialize");
        assert_eq!(
            value,
            json!({
                "version": 0,
                "id": "s1",
                "createdAt": 1,
                "cwd": "/work",
                "parentSession": "parent",
                "seedLength": 3,
                "origin": "subagent",
                "delegationDepth": 1,
                "agentPreset": "standard",
            })
        );
        let back: SessionHeader = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back, header);
    }

    #[test]
    fn header_omits_absent_optionals() {
        let header = SessionHeader {
            version: 0,
            id: SessionId::new("s1"),
            created_at: 0,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        };
        let value = serde_json::to_value(&header).expect("serialize");
        let obj = value.as_object().expect("object");
        assert!(!obj.contains_key("cwd"));
        assert!(!obj.contains_key("parentSession"));
        assert!(!obj.contains_key("origin"));
    }
}
