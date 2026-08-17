//! Model-facing `mcp__` public tool names.

use sha2::{Digest, Sha256};

/// DeepSeek function-name maximum length. Wire-protocol constant, not configuration.
const MAX_PUBLIC_NAME_LENGTH: usize = 64;

/// Hex characters of the SHA-256 identity hash appended on lossy normalization.
const HASH_HEX_LEN: usize = 12;

/// Derive the model-facing public name for one MCP tool.
///
/// # Parameters
///
/// * `server_name` - Local YAML `serverName`, not remote `serverInfo.name`.
/// * `raw_name` - The MCP server's own tool name, sent on `tools/call`.
///
/// # Returns
///
/// `mcp__<serverName>__<rawName>` when that string already matches
/// `[A-Za-z0-9_-]` and is at most 64 characters. Otherwise the charset-normalized
/// form is truncated to 51 characters and `_` plus a 12-hex SHA-256 of
/// `serverName + NUL + rawName` is appended.
#[must_use]
pub fn public_tool_name(server_name: &str, raw_name: &str) -> String {
    let joined = format!("mcp__{server_name}__{raw_name}");
    let normalized: String = joined
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if normalized == joined && normalized.len() <= MAX_PUBLIC_NAME_LENGTH {
        return joined;
    }
    let hash = identity_hash(server_name, raw_name);
    let prefix: String = normalized
        .chars()
        .take(MAX_PUBLIC_NAME_LENGTH - HASH_HEX_LEN - 1)
        .collect();
    format!("{prefix}_{hash}")
}

fn identity_hash(server_name: &str, raw_name: &str) -> String {
    let digest = Sha256::digest(format!("{server_name}\0{raw_name}").as_bytes());
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(digest.len() * 2);
    for &byte in digest.as_slice() {
        hex.push(DIGITS[(byte >> 4) as usize] as char);
        hex.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex[..HASH_HEX_LEN].to_string()
}

#[cfg(test)]
mod tests {
    use super::public_tool_name;

    #[test]
    fn public_tool_name_clean_github_create_issue() {
        assert_eq!(
            public_tool_name("github", "create_issue"),
            "mcp__github__create_issue"
        );
    }

    #[test]
    fn public_tool_name_preserves_hyphen_in_get_sum() {
        assert_eq!(
            public_tool_name("everything", "get-sum"),
            "mcp__everything__get-sum"
        );
    }

    #[test]
    fn public_tool_name_dotted_admin_reset_hashes() {
        assert_eq!(
            public_tool_name("srv", "admin.reset"),
            "mcp__srv__admin_reset_3b185f786768"
        );
    }

    #[test]
    fn public_tool_name_underscore_admin_reset_unchanged() {
        assert_eq!(
            public_tool_name("srv", "admin_reset"),
            "mcp__srv__admin_reset"
        );
    }

    #[test]
    fn public_tool_name_long_raw_truncates_to_64_with_hash() {
        let name = public_tool_name("srv", &"a".repeat(80));
        assert_eq!(name.len(), 64);
        assert!(name.ends_with("_3b75b5cc78d8"));
    }

    #[test]
    fn public_tool_name_fixture_admin_reset_vector() {
        assert_eq!(
            public_tool_name("fixture", "admin.reset"),
            "mcp__fixture__admin_reset_2d9bb2dfe9aa"
        );
    }
}
