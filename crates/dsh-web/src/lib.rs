//! Web search capability: provider registry, selection, and search execution.

pub mod plugin;
mod runtime;

#[cfg(test)]
mod phase6_exit;

pub use plugin::register;
pub use runtime::{WebRuntime, WebSearchProvider};

/// Duplicate provider id within search.
pub const WEB_DUPLICATE_PROVIDER: &str = "WEB_DUPLICATE_PROVIDER";
/// Configured provider id is not registered.
pub const WEB_PROVIDER_CONFIGURED_MISSING: &str = "WEB_PROVIDER_CONFIGURED_MISSING";
/// Configured provider is registered but `available()` is false.
pub const WEB_PROVIDER_CONFIGURED_UNAVAILABLE: &str = "WEB_PROVIDER_CONFIGURED_UNAVAILABLE";
/// No configured id and more than one usable provider.
pub const WEB_PROVIDER_AMBIGUOUS: &str = "WEB_PROVIDER_AMBIGUOUS";
/// No configured id and no usable provider.
pub const WEB_PROVIDER_UNAVAILABLE: &str = "WEB_PROVIDER_UNAVAILABLE";
/// Caller cancelled the search.
pub const WEB_ABORTED: &str = "WEB_ABORTED";
/// Selected provider failed the search.
pub const WEB_PROVIDER_ERROR: &str = "WEB_PROVIDER_ERROR";
/// Selected provider has no usable API key at search time.
pub const WEB_PROVIDER_CREDENTIAL_MISSING: &str = "WEB_PROVIDER_CREDENTIAL_MISSING";

/// One search request. `max_results` is enforced by [`WebRuntime::search`] on the way back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebSearchRequest {
    /// Search query text.
    pub query: String,
    /// Upper bound on returned sources; omitted means no bound.
    pub max_results: Option<u32>,
}

/// One citeable source. `url` is required; the other fields are omitted when the provider has none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebSearchSource {
    /// Source URL.
    pub url: String,
    /// Optional title.
    pub title: Option<String>,
    /// Optional excerpt.
    pub snippet: Option<String>,
    /// Optional publication or crawl timestamp as a provider-supplied string.
    pub published_at: Option<String>,
}

/// Normalized search outcome. `truncated` is true when the provider flagged truncation or the runtime capped `sources`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebSearchResult {
    /// Optional provider-generated answer or summary.
    pub content: Option<String>,
    /// Citeable sources, already truncated to `max_results` when that bound was set.
    pub sources: Vec<WebSearchSource>,
    /// True when sources were cut to honor `max_results`, or the provider already set this flag.
    pub truncated: bool,
}

/// Typed web failure. [`Display`](std::fmt::Display) is `message`; `code` is the stable routing token.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct WebError {
    /// Human-readable failure summary.
    pub message: String,
    /// Stable machine-routing code such as [`WEB_PROVIDER_UNAVAILABLE`].
    pub code: String,
}

impl WebError {
    /// Construct a failure with `message` and `code`.
    #[must_use]
    pub fn new(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: code.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WEB_PROVIDER_UNAVAILABLE, WebError};

    #[test]
    fn web_error_displays_message() {
        let err = WebError::new(
            "no usable web provider is registered",
            WEB_PROVIDER_UNAVAILABLE,
        );
        assert_eq!(err.to_string(), "no usable web provider is registered");
        assert_eq!(err.code, WEB_PROVIDER_UNAVAILABLE);
    }
}
