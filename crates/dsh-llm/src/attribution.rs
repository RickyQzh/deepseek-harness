//! Product identity sent on every provider HTTP request.

/// Standard `User-Agent` value for this crate's package version.
#[must_use]
pub fn user_agent() -> String {
    format!(
        "deepseek-harness/{} (+https://github.com/deepseek-ai/deepseek-harness)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Attribution headers an adapter must send on every provider request.
///
/// Header names are lowercase. Currently this is only `user-agent`.
#[must_use]
pub fn attribution_headers() -> Vec<(String, String)> {
    vec![("user-agent".into(), user_agent())]
}

#[cfg(test)]
mod tests {
    use super::{attribution_headers, user_agent};

    #[test]
    fn user_agent_uses_crate_version() {
        let expected = format!(
            "deepseek-harness/{} (+https://github.com/deepseek-ai/deepseek-harness)",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(user_agent(), expected);
        assert_eq!(attribution_headers(), vec![("user-agent".into(), expected)]);
    }
}
