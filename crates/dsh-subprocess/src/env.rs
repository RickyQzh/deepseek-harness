//! Credential-scrubbed child environments.

/// Namespace prefix reserved for DeepSeek Harness-managed child environment facts.
pub const DSH_ENV_PREFIX: &str = "DSH_";

/// One explicit child-environment overlay entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvEntry {
    /// Environment key.
    pub key: String,
    /// `Some` sets/overrides; `None` removes an ambient key.
    pub value: Option<String>,
}

/// Whether `name` matches the credential-shaped substring heuristic (`KEY`, `PASSWORD`, `SECRET`, or `TOKEN`, ASCII-case-insensitive).
#[must_use]
pub fn sensitive_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("KEY")
        || upper.contains("PASSWORD")
        || upper.contains("SECRET")
        || upper.contains("TOKEN")
}

/// Ambient parent environment minus credential-shaped names and minus names whose ASCII-uppercased form starts with [`DSH_ENV_PREFIX`].
#[must_use]
pub fn scrubbed_parent_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(key, _)| {
            !sensitive_env_name(key) && !key.to_ascii_uppercase().starts_with(DSH_ENV_PREFIX)
        })
        .collect()
}

/// Build the child environment from [`scrubbed_parent_env`], then overlay `extra`.
///
/// `None` leaves the scrubbed parent unchanged. A `Some` value sets or replaces that exact key; a `None` value is a tombstone that removes it. Later exact keys replace earlier ones (POSIX last-key-wins).
#[must_use]
pub fn child_env(extra: Option<&[EnvEntry]>) -> Vec<(String, String)> {
    let mut env = scrubbed_parent_env();
    if let Some(extra) = extra {
        for entry in extra {
            env.retain(|(k, _)| k != &entry.key);
            if let Some(value) = &entry.value {
                env.push((entry.key.clone(), value.clone()));
            }
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::{DSH_ENV_PREFIX, EnvEntry, child_env, scrubbed_parent_env, sensitive_env_name};

    #[test]
    fn dsh_prefix_is_dsh_underscore() {
        assert_eq!(DSH_ENV_PREFIX, "DSH_");
    }

    #[test]
    fn sensitive_pattern_matches_key_password_secret_token_case_insensitively() {
        for name in [
            "DEEPSEEK_API_KEY",
            "password",
            "Secret",
            "AUTH_TOKEN",
            "apiKey",
        ] {
            assert!(sensitive_env_name(name), "{name}");
        }
        assert!(!sensitive_env_name("PATH"));
        assert!(!sensitive_env_name("HOME"));
        assert!(!sensitive_env_name("LANG"));
    }

    #[test]
    fn scrubbed_parent_env_drops_credentials_and_dsh_names() {
        let previous_key = std::env::var("DEEPSEEK_API_KEY").ok();
        let previous_dsh = std::env::var("DSH_PHASE4_SCRUB").ok();
        unsafe {
            std::env::set_var("DEEPSEEK_API_KEY", "secret-value");
            std::env::set_var("DSH_PHASE4_SCRUB", "nope");
        }
        let env = scrubbed_parent_env();
        assert!(env.iter().all(|(k, _)| k != "DEEPSEEK_API_KEY"));
        assert!(env.iter().all(|(k, _)| k != "DSH_PHASE4_SCRUB"));
        assert!(env.iter().any(|(k, _)| k == "PATH"));
        match previous_key {
            Some(value) => unsafe { std::env::set_var("DEEPSEEK_API_KEY", value) },
            None => unsafe { std::env::remove_var("DEEPSEEK_API_KEY") },
        }
        match previous_dsh {
            Some(value) => unsafe { std::env::set_var("DSH_PHASE4_SCRUB", value) },
            None => unsafe { std::env::remove_var("DSH_PHASE4_SCRUB") },
        }
    }

    #[test]
    fn explicit_overlay_can_restore_a_credential_and_tombstone_removes_ambient() {
        let previous = std::env::var("PATH").ok();
        let env = child_env(Some(&[
            EnvEntry {
                key: "DEEPSEEK_API_KEY".into(),
                value: Some("deliberate".into()),
            },
            EnvEntry {
                key: "PATH".into(),
                value: None,
            },
        ]));
        assert_eq!(
            env.iter()
                .find(|(k, _)| k == "DEEPSEEK_API_KEY")
                .map(|(_, v)| v.as_str()),
            Some("deliberate")
        );
        assert!(env.iter().all(|(k, _)| k != "PATH"));
        assert!(previous.is_some());
    }
}
