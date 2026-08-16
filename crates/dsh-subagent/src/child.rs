//! Delegation-depth accounting shared by in-process child composition.

use dsh_session::SessionHeader;

use crate::SubagentError;

/// Default absolute cap when [`crate::SubagentStartRequest::max_depth`] is omitted.
pub const DEFAULT_SUBAGENT_MAX_DEPTH: u32 = 3;

/// Read persisted delegation depth, treating absence as top-level depth zero.
#[must_use]
pub fn delegation_depth_of(header: &SessionHeader) -> u64 {
    header.delegation_depth.unwrap_or(0)
}

/// Resolve the child's depth as parent depth plus one and reject an exceeded cap.
///
/// `max_depth` omitted means [`DEFAULT_SUBAGENT_MAX_DEPTH`].
///
/// # Errors
///
/// [`SubagentError::Depth`] when `parent_depth + 1` is greater than the cap.
pub fn assert_subagent_max_depth(
    parent_depth: u64,
    max_depth: Option<u32>,
) -> Result<u64, SubagentError> {
    let max = u64::from(max_depth.unwrap_or(DEFAULT_SUBAGENT_MAX_DEPTH));
    let child_depth = parent_depth.saturating_add(1);
    if child_depth > max {
        return Err(SubagentError::Depth(child_depth, max as u32));
    }
    Ok(child_depth)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SUBAGENT_MAX_DEPTH, assert_subagent_max_depth, delegation_depth_of};
    use crate::SubagentError;
    use dsh_session::{SESSION_FORMAT_VERSION, SessionHeader, SessionId};

    fn header_with_depth(depth: Option<u64>) -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("p"),
            created_at: 1,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: depth,
            agent_preset: None,
        }
    }

    #[test]
    fn absent_header_depth_is_zero() {
        assert_eq!(delegation_depth_of(&header_with_depth(None)), 0);
        assert_eq!(delegation_depth_of(&header_with_depth(Some(2))), 2);
    }

    #[test]
    fn default_cap_allows_top_level_child() {
        assert_eq!(assert_subagent_max_depth(0, None).unwrap(), 1);
        assert_eq!(
            assert_subagent_max_depth(2, None).unwrap(),
            DEFAULT_SUBAGENT_MAX_DEPTH as u64
        );
    }

    #[test]
    fn exceeded_cap_fails_loud() {
        let err = assert_subagent_max_depth(3, None).unwrap_err();
        assert!(matches!(err, SubagentError::Depth(4, 3)));
        assert!(err.to_string().contains("exceeds maxDepth 3"));
    }
}
