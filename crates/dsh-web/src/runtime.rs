//! Search provider registry and execution-time selection.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use dsh_tools::AbortFlag;

use crate::{
    WEB_ABORTED, WEB_DUPLICATE_PROVIDER, WEB_PROVIDER_AMBIGUOUS, WEB_PROVIDER_CONFIGURED_MISSING,
    WEB_PROVIDER_CONFIGURED_UNAVAILABLE, WEB_PROVIDER_UNAVAILABLE, WebError, WebSearchRequest,
    WebSearchResult,
};

/// A search-capable backend registered with [`WebRuntime::register_search_provider`].
pub trait WebSearchProvider: Send + Sync {
    /// Stable id, unique among search providers on one runtime.
    fn id(&self) -> &str;
    /// Cheap local usability check; must not make network calls.
    fn available(&self) -> bool;
    /// Run one search and honor `signal` for cancellation.
    ///
    /// The returned future borrows `self`.
    fn search<'a>(
        &'a self,
        request: WebSearchRequest,
        signal: AbortFlag,
    ) -> Pin<Box<dyn Future<Output = Result<WebSearchResult, WebError>> + Send + 'a>>;
}

/// Search provider registry. `inject::<WebRuntime>()` yields `Arc<WebRuntime>`, so registration uses an interior mutex.
pub struct WebRuntime {
    search_providers: Mutex<HashMap<String, Arc<dyn WebSearchProvider>>>,
    search_provider_id: Option<String>,
}

impl WebRuntime {
    /// Empty registry. `search_provider_id` pins the search provider when `Some`.
    #[must_use]
    pub fn new(search_provider_id: Option<String>) -> Self {
        Self {
            search_providers: Mutex::new(HashMap::new()),
            search_provider_id,
        }
    }

    /// Register a search provider. Duplicate [`WebSearchProvider::id`] values fail.
    ///
    /// # Errors
    ///
    /// [`WEB_DUPLICATE_PROVIDER`] when a provider with that id is already registered.
    pub fn register_search_provider(
        &self,
        provider: Arc<dyn WebSearchProvider>,
    ) -> Result<(), WebError> {
        let mut providers = self
            .search_providers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = provider.id();
        if providers.contains_key(id) {
            return Err(WebError::new(
                format!("a web provider with id \"{id}\" is already registered"),
                WEB_DUPLICATE_PROVIDER,
            ));
        }
        providers.insert(id.to_string(), provider);
        Ok(())
    }

    /// Resolve the search provider and run one search. Caps `sources` to `request.max_results`.
    ///
    /// # Errors
    ///
    /// Selection failures (`WEB_PROVIDER_*`), [`WEB_ABORTED`] when `signal` is aborted, or the provider's [`WebError`].
    pub async fn search(
        &self,
        request: WebSearchRequest,
        signal: AbortFlag,
    ) -> Result<WebSearchResult, WebError> {
        if signal.is_aborted() {
            return Err(aborted());
        }
        let provider = self.resolve_search_provider()?;
        if signal.is_aborted() {
            return Err(aborted());
        }
        let max_results = request.max_results;
        let result = provider.search(request, signal.clone()).await?;
        if signal.is_aborted() {
            return Err(aborted());
        }
        Ok(cap_sources(result, max_results))
    }

    fn resolve_search_provider(&self) -> Result<Arc<dyn WebSearchProvider>, WebError> {
        let providers = self
            .search_providers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(configured_id) = &self.search_provider_id {
            let Some(provider) = providers.get(configured_id) else {
                return Err(WebError::new(
                    format!("configured web provider \"{configured_id}\" is not registered"),
                    WEB_PROVIDER_CONFIGURED_MISSING,
                ));
            };
            if !provider.available() {
                return Err(WebError::new(
                    format!(
                        "configured web provider \"{configured_id}\" is registered but unavailable"
                    ),
                    WEB_PROVIDER_CONFIGURED_UNAVAILABLE,
                ));
            }
            return Ok(Arc::clone(provider));
        }
        let mut usable: Vec<Arc<dyn WebSearchProvider>> = providers
            .values()
            .filter(|provider| provider.available())
            .cloned()
            .collect();
        match usable.len() {
            0 => Err(WebError::new(
                "no usable web provider is registered",
                WEB_PROVIDER_UNAVAILABLE,
            )),
            1 => Ok(usable.remove(0)),
            _ => {
                let mut ids: Vec<&str> = usable.iter().map(|provider| provider.id()).collect();
                ids.sort_unstable();
                Err(WebError::new(
                    format!(
                        "multiple usable web providers are registered ({}); configure one explicitly",
                        ids.join(", ")
                    ),
                    WEB_PROVIDER_AMBIGUOUS,
                ))
            }
        }
    }
}

fn aborted() -> WebError {
    WebError::new("web search aborted", WEB_ABORTED)
}

fn cap_sources(result: WebSearchResult, max_results: Option<u32>) -> WebSearchResult {
    let Some(max) = max_results else {
        return result;
    };
    let max = usize::try_from(max).unwrap_or(usize::MAX);
    if result.sources.len() <= max {
        return result;
    }
    WebSearchResult {
        content: result.content,
        sources: result.sources.into_iter().take(max).collect(),
        truncated: true,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dsh_tools::AbortFlag;

    use super::WebRuntime;
    use crate::{
        WEB_DUPLICATE_PROVIDER, WEB_PROVIDER_AMBIGUOUS, WEB_PROVIDER_CONFIGURED_MISSING,
        WEB_PROVIDER_CONFIGURED_UNAVAILABLE, WEB_PROVIDER_UNAVAILABLE, WebError, WebSearchProvider,
        WebSearchRequest, WebSearchResult, WebSearchSource,
    };

    struct FakeSearch {
        n: usize,
    }

    impl WebSearchProvider for FakeSearch {
        fn id(&self) -> &str {
            "fake"
        }

        fn available(&self) -> bool {
            true
        }

        fn search<'a>(
            &'a self,
            _request: WebSearchRequest,
            _signal: AbortFlag,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<WebSearchResult, WebError>> + Send + 'a>,
        > {
            let n = self.n;
            Box::pin(async move {
                let sources = (1..=n)
                    .map(|index| WebSearchSource {
                        url: format!("https://{index}"),
                        title: None,
                        snippet: None,
                        published_at: None,
                    })
                    .collect();
                Ok(WebSearchResult {
                    content: None,
                    sources,
                    truncated: false,
                })
            })
        }
    }

    struct ScriptedSearch {
        id: &'static str,
        available: bool,
        truncated: bool,
        sources: Vec<WebSearchSource>,
    }

    impl WebSearchProvider for ScriptedSearch {
        fn id(&self) -> &str {
            self.id
        }

        fn available(&self) -> bool {
            self.available
        }

        fn search<'a>(
            &'a self,
            _request: WebSearchRequest,
            _signal: AbortFlag,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<WebSearchResult, WebError>> + Send + 'a>,
        > {
            let sources = self.sources.clone();
            let truncated = self.truncated;
            Box::pin(async move {
                Ok(WebSearchResult {
                    content: Some(self.id.to_string()),
                    sources,
                    truncated,
                })
            })
        }
    }

    fn source(url: &str) -> WebSearchSource {
        WebSearchSource {
            url: url.into(),
            title: None,
            snippet: None,
            published_at: None,
        }
    }

    #[tokio::test]
    #[allow(unused_mut)]
    async fn search_caps_sources_and_sets_truncated() {
        let mut web = WebRuntime::new(None);
        web.register_search_provider(Arc::new(FakeSearch { n: 3 }))
            .unwrap();
        let result = web
            .search(
                WebSearchRequest {
                    query: "q".into(),
                    max_results: Some(1),
                },
                AbortFlag::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.sources.len(), 1);
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn duplicate_id_is_web_duplicate_provider() {
        let web = WebRuntime::new(None);
        web.register_search_provider(Arc::new(FakeSearch { n: 1 }))
            .unwrap();
        let err = web
            .register_search_provider(Arc::new(FakeSearch { n: 2 }))
            .unwrap_err();
        assert_eq!(err.code, WEB_DUPLICATE_PROVIDER);
        assert!(err.to_string().contains("\"fake\""));
    }

    #[tokio::test]
    async fn configured_missing_id_fails() {
        let web = WebRuntime::new(Some("perplexity".into()));
        web.register_search_provider(Arc::new(FakeSearch { n: 1 }))
            .unwrap();
        let err = web
            .search(
                WebSearchRequest {
                    query: "q".into(),
                    max_results: None,
                },
                AbortFlag::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, WEB_PROVIDER_CONFIGURED_MISSING);
    }

    #[tokio::test]
    async fn configured_unavailable_id_fails() {
        let web = WebRuntime::new(Some("exa".into()));
        web.register_search_provider(Arc::new(ScriptedSearch {
            id: "exa",
            available: false,
            truncated: false,
            sources: vec![],
        }))
        .unwrap();
        let err = web
            .search(
                WebSearchRequest {
                    query: "q".into(),
                    max_results: None,
                },
                AbortFlag::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, WEB_PROVIDER_CONFIGURED_UNAVAILABLE);
    }

    #[tokio::test]
    async fn none_and_many_usable_is_ambiguous() {
        let web = WebRuntime::new(None);
        web.register_search_provider(Arc::new(ScriptedSearch {
            id: "exa",
            available: true,
            truncated: false,
            sources: vec![],
        }))
        .unwrap();
        web.register_search_provider(Arc::new(ScriptedSearch {
            id: "perplexity",
            available: true,
            truncated: false,
            sources: vec![],
        }))
        .unwrap();
        let err = web
            .search(
                WebSearchRequest {
                    query: "q".into(),
                    max_results: None,
                },
                AbortFlag::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, WEB_PROVIDER_AMBIGUOUS);
        assert!(err.to_string().contains("exa"));
        assert!(err.to_string().contains("perplexity"));
    }

    #[tokio::test]
    async fn none_and_zero_usable_is_unavailable() {
        let web = WebRuntime::new(None);
        let err = web
            .search(
                WebSearchRequest {
                    query: "q".into(),
                    max_results: None,
                },
                AbortFlag::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, WEB_PROVIDER_UNAVAILABLE);
    }

    #[tokio::test]
    async fn none_and_one_usable_selects_it() {
        let web = WebRuntime::new(None);
        web.register_search_provider(Arc::new(ScriptedSearch {
            id: "exa",
            available: true,
            truncated: false,
            sources: vec![source("https://a")],
        }))
        .unwrap();
        web.register_search_provider(Arc::new(ScriptedSearch {
            id: "down",
            available: false,
            truncated: false,
            sources: vec![],
        }))
        .unwrap();
        let result = web
            .search(
                WebSearchRequest {
                    query: "q".into(),
                    max_results: None,
                },
                AbortFlag::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.content.as_deref(), Some("exa"));
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn provider_truncated_is_kept_when_within_cap() {
        let web = WebRuntime::new(None);
        web.register_search_provider(Arc::new(ScriptedSearch {
            id: "exa",
            available: true,
            truncated: true,
            sources: vec![source("https://a")],
        }))
        .unwrap();
        let result = web
            .search(
                WebSearchRequest {
                    query: "q".into(),
                    max_results: Some(8),
                },
                AbortFlag::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.sources.len(), 1);
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn aborted_signal_fails_before_provider() {
        let web = WebRuntime::new(None);
        web.register_search_provider(Arc::new(FakeSearch { n: 1 }))
            .unwrap();
        let signal = AbortFlag::new();
        signal.abort();
        let err = web
            .search(
                WebSearchRequest {
                    query: "q".into(),
                    max_results: None,
                },
                signal,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, crate::WEB_ABORTED);
    }
}
