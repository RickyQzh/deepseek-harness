//! `type: "session"` header line for a JSONL session log.

use serde::{Deserialize, Serialize};

use crate::PersistError;
use dsh_session::{SessionHeader, SessionId, SessionOrigin};

/// Discriminator for the first JSONL record of a session log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeaderLineType {
    /// The session-header record.
    #[serde(rename = "session")]
    Session,
}

/// First JSONL record: immutable session metadata tagged `type: "session"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderLine {
    /// Record discriminator.
    #[serde(rename = "type")]
    pub kind: HeaderLineType,
    /// On-disk format version.
    pub version: i64,
    /// Session identity.
    pub id: SessionId,
    /// Unix epoch milliseconds when the session was created.
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    /// Working directory recorded at creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Parent session when this session was forked.
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
    /// Delegation depth; always written, `0` for a top-level session.
    #[serde(rename = "delegationDepth")]
    pub delegation_depth: u64,
    /// Agent preset id this session was composed from, when recorded.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "agentPreset"
    )]
    pub agent_preset: Option<String>,
}

/// Build the on-disk header record from in-memory metadata.
///
/// `delegation_depth` is always written; a missing in-memory value becomes `0`.
#[must_use]
pub fn to_header_line(header: &SessionHeader) -> HeaderLine {
    HeaderLine {
        kind: HeaderLineType::Session,
        version: i64::from(header.version),
        id: header.id.clone(),
        created_at: header.created_at,
        cwd: header.cwd.clone(),
        parent_session: header.parent_session.clone(),
        seed_length: header.seed_length,
        origin: header.origin.clone(),
        delegation_depth: header.delegation_depth.unwrap_or(0),
        agent_preset: header.agent_preset.clone(),
    }
}

/// Convert a shape-checked header record back to in-memory metadata.
///
/// On-disk `delegationDepth` `0` becomes `Some(0)` so a header that stored
/// zero round-trips.
///
/// # Errors
///
/// [`PersistError::Corrupt`] when `version` does not fit in [`u32`].
pub fn from_header_line(line: HeaderLine) -> Result<SessionHeader, PersistError> {
    let version = u32::try_from(line.version).map_err(|_| {
        PersistError::Corrupt("corrupt session log: first line is not a session header".into())
    })?;
    Ok(SessionHeader {
        version,
        id: line.id,
        created_at: line.created_at,
        cwd: line.cwd,
        parent_session: line.parent_session,
        seed_length: line.seed_length,
        origin: line.origin,
        delegation_depth: Some(line.delegation_depth),
        agent_preset: line.agent_preset,
    })
}

/// Parse the first JSONL line as a session header.
///
/// [`dsh_session::refuse_foreign_format_version`] runs on the parsed JSON
/// before `HeaderLine` shape validation so a newer format is refused as
/// unsupported, not corrupt.
///
/// # Errors
///
/// [`PersistError::Format`] when `version` is present and not
/// [`dsh_session::SESSION_FORMAT_VERSION`].
/// [`PersistError::Corrupt`] when the line is not valid JSON, uses retired
/// `sandboxMode` / `approvalPolicy` fields, or is not a session header.
pub fn parse_header_record(line: &str) -> Result<SessionHeader, PersistError> {
    let parsed: serde_json::Value = serde_json::from_str(line).map_err(|_| {
        PersistError::Corrupt("corrupt session log: header line is not valid JSON".into())
    })?;
    dsh_session::refuse_foreign_format_version(&parsed)?;
    if parsed.as_object().is_some_and(|object| {
        object.contains_key("sandboxMode") || object.contains_key("approvalPolicy")
    }) {
        return Err(PersistError::Corrupt(
            "session header uses retired policy baseline fields".into(),
        ));
    }
    let line: HeaderLine = serde_json::from_value(parsed).map_err(|_| {
        PersistError::Corrupt("corrupt session log: first line is not a session header".into())
    })?;
    from_header_line(line)
}

#[cfg(test)]
mod tests {
    use super::{parse_header_record, to_header_line};
    use dsh_session::{SESSION_FORMAT_VERSION, SessionHeader, SessionId};

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
    fn to_header_line_always_writes_delegation_depth() {
        let line = to_header_line(&header());
        assert_eq!(line.delegation_depth, 0);
        let json = serde_json::to_string(&line).expect("json");
        assert!(json.contains("\"type\":\"session\""));
        assert!(json.contains("\"delegationDepth\":0"));
    }

    #[test]
    fn retired_policy_fields_are_corrupt() {
        let error = parse_header_record(
            r#"{"type":"session","version":0,"id":"s","createdAt":1,"delegationDepth":0,"sandboxMode":"read-only"}"#,
        )
        .expect_err("retired");
        assert!(error.to_string().contains("retired policy baseline fields"));
    }

    #[test]
    fn from_header_line_round_trips_delegation_depth_zero() {
        let parsed =
            parse_header_record(&serde_json::to_string(&to_header_line(&header())).expect("json"))
                .expect("parse");
        assert_eq!(parsed.delegation_depth, Some(0));
        assert_eq!(parsed.id.as_str(), "s1");
    }
}
