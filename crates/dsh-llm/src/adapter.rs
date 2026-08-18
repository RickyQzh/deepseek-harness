//! Adapter trait, registry, and stream wrap that turns adapter failures into finish chunks.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use dsh_session::{FinishReason, LlmCallConfig, LlmCallConfigAdapterDefaults, StreamChunk};
use futures::Stream;

use crate::error::{ABORTED_CODE, LlmError};
use crate::retry::ResolvedRetryPolicy;
use crate::types::{GenerateOptions, LlmModelContext, LlmProviderInfo, LlmResolvedModelInfo};

/// Provider-wire adapter for the harness message and stream vocabulary.
///
/// `stream` may yield [`Err`](Result::Err) for dispatch or iteration failure.
/// [`LlmRuntime::stream`] converts that error into a terminal [`StreamChunk::Finish`].
pub trait LlmAdapter: Send + Sync {
    /// Describe one provider route owned by this adapter.
    fn provider_info(&self, provider: &str) -> LlmProviderInfo {
        LlmProviderInfo {
            id: provider.into(),
            name: provider.into(),
        }
    }

    /// Resolve metadata available for one exact model.
    fn resolve_model(&self, provider: &str, model: &str) -> LlmResolvedModelInfo {
        LlmResolvedModelInfo {
            provider: provider.into(),
            id: model.into(),
            name: model.into(),
            context: None,
            default_max_tokens: None,
            reasoning: None,
        }
    }

    /// Stream one model call as raw chunks, or an internal error for the runtime wrap.
    fn stream(
        &self,
        options: GenerateOptions,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LlmError>> + Send + '_>>;

    /// Provider-owned retry policy for this adapter's route.
    fn retry_policy(&self) -> ResolvedRetryPolicy {
        ResolvedRetryPolicy::default()
    }
}

/// One model call whose config and adapter registration were resolved together.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedLlmCall {
    /// Config with any adapter-owned default materialized.
    pub config: LlmCallConfig,
    /// Markers for fields supplied by adapter defaults.
    pub adapter_defaults: LlmCallConfigAdapterDefaults,
    /// Detached context metadata resolved with the registration-bound call.
    pub context: Option<LlmModelContext>,
}

/// Adapter registry. [`Self::stream`] never returns a thrown error to the consumer.
pub struct LlmRuntime {
    adapters: HashMap<String, Arc<dyn LlmAdapter>>,
}

impl Default for LlmRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for LlmRuntime {
    fn clone(&self) -> Self {
        Self {
            adapters: self.adapters.clone(),
        }
    }
}

impl LlmRuntime {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    /// Provider route ids currently registered.
    #[must_use]
    pub fn list_providers(&self) -> Vec<String> {
        self.adapters.keys().cloned().collect()
    }

    /// Register `adapter` for `provider`, replacing any previous adapter on that route.
    pub fn register_adapter(&mut self, provider: impl Into<String>, adapter: Arc<dyn LlmAdapter>) {
        self.adapters.insert(provider.into(), adapter);
    }

    /// Resolved retry policy for `provider`, or [`ResolvedRetryPolicy::default`] when unregistered.
    #[must_use]
    pub fn provider_retry_policy(&self, provider: &str) -> ResolvedRetryPolicy {
        match self.adapters.get(provider) {
            Some(adapter) => adapter.retry_policy(),
            None => ResolvedRetryPolicy::default(),
        }
    }

    /// Look up the adapter, resolve the model, and materialize omitted adapter defaults.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError`] with code `NO_ADAPTER` when `config.provider` is unregistered.
    pub fn prepare_call(&self, config: &LlmCallConfig) -> Result<PreparedLlmCall, LlmError> {
        let Some(adapter) = self.adapters.get(&config.provider) else {
            return Err(LlmError::new(
                format!("no adapter registered for provider \"{}\"", config.provider),
                "NO_ADAPTER",
            ));
        };
        let info = adapter.resolve_model(&config.provider, &config.model);
        let mut prepared = config.clone();
        let mut adapter_defaults = LlmCallConfigAdapterDefaults::default();
        if prepared.max_tokens.is_none() {
            if let Some(default_max_tokens) = info.default_max_tokens {
                prepared.max_tokens = Some(default_max_tokens);
                adapter_defaults.max_tokens = Some(true);
            }
        }
        if prepared.reasoning_effort.is_none() {
            if let Some(default_effort) = info
                .reasoning
                .as_ref()
                .and_then(|reasoning| reasoning.default_effort.clone())
            {
                prepared.reasoning_effort = Some(default_effort);
                adapter_defaults.reasoning_effort = Some(true);
            }
        }
        Ok(PreparedLlmCall {
            config: prepared,
            adapter_defaults,
            context: info.context,
        })
    }

    /// Stream one model call. Adapter failures become a terminal finish chunk.
    pub fn stream(
        &self,
        options: GenerateOptions,
    ) -> Pin<Box<dyn Stream<Item = StreamChunk> + Send + '_>> {
        let aborted = options.signal.is_aborted();
        let Some(adapter) = self.adapters.get(&options.provider) else {
            let failure = LlmError::new(
                format!(
                    "no adapter registered for provider \"{}\"",
                    options.provider
                ),
                "NO_ADAPTER",
            );
            return Box::pin(futures::stream::iter([finish_from_error(failure, aborted)]));
        };
        let inner = adapter.stream(options);
        Box::pin(AdapterWrap {
            inner,
            aborted,
            done: false,
        })
    }
}

fn finish_from_error(error: LlmError, signal_aborted: bool) -> StreamChunk {
    let aborted = signal_aborted || error.code == ABORTED_CODE;
    let failure = error.failure();
    StreamChunk::Finish {
        reason: if aborted {
            FinishReason::Aborted { failure }
        } else {
            FinishReason::Error { failure }
        },
        replay_state: None,
    }
}

struct AdapterWrap<'a> {
    inner: Pin<Box<dyn Stream<Item = Result<StreamChunk, LlmError>> + Send + 'a>>,
    aborted: bool,
    done: bool,
}

impl Stream for AdapterWrap<'_> {
    type Item = StreamChunk;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        match this.inner.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(chunk))) => {
                if matches!(chunk, StreamChunk::Finish { .. }) {
                    this.done = true;
                }
                Poll::Ready(Some(chunk))
            }
            Poll::Ready(Some(Err(error))) => {
                this.done = true;
                Poll::Ready(Some(finish_from_error(error, this.aborted)))
            }
            Poll::Ready(None) => {
                this.done = true;
                Poll::Ready(Some(StreamChunk::Finish {
                    reason: FinishReason::Stop,
                    replay_state: None,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LlmAdapter, LlmRuntime};
    use crate::mock::{MockAdapter, MockScript, text_response};
    use crate::{GenerateOptions, LlmError};
    use dsh_session::{FinishReason, StreamChunk};
    use dsh_tools::AbortFlag;
    use futures::StreamExt;
    use std::sync::Arc;

    fn options(adapter: &str) -> GenerateOptions {
        GenerateOptions {
            provider: adapter.into(),
            model: "mock".into(),
            reasoning_effort: None,
            messages: vec![],
            system: None,
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
            signal: AbortFlag::new(),
            session_id: None,
            purpose: None,
        }
    }

    async fn collect(runtime: &LlmRuntime, options: GenerateOptions) -> Vec<StreamChunk> {
        let mut stream = runtime.stream(options);
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            out.push(chunk);
        }
        out
    }

    #[tokio::test]
    async fn usage_arrives_before_finish_and_nothing_after() {
        let adapter: Arc<dyn LlmAdapter> = Arc::new(MockAdapter::new(vec![MockScript::Chunks(
            text_response("hi"),
        )]));
        let mut runtime = LlmRuntime::new();
        runtime.register_adapter("mock", adapter);
        let chunks = collect(&runtime, options("mock")).await;
        let types: Vec<_> = chunks
            .iter()
            .map(|c| match c {
                StreamChunk::Usage { .. } => "usage",
                StreamChunk::Finish { .. } => "finish",
                _ => "other",
            })
            .collect();
        let usage_at = types.iter().position(|t| *t == "usage").expect("usage");
        let finish_at = types.iter().position(|t| *t == "finish").expect("finish");
        assert!(usage_at < finish_at);
        assert_eq!(finish_at, types.len() - 1);
    }

    #[tokio::test]
    async fn adapter_failure_is_terminal_finish_not_a_thrown_error() {
        let adapter: Arc<dyn LlmAdapter> = Arc::new(MockAdapter::new(vec![MockScript::Fail(
            LlmError::new("boom", "SERVER"),
        )]));
        let mut runtime = LlmRuntime::new();
        runtime.register_adapter("mock", adapter);
        let chunks = collect(&runtime, options("mock")).await;
        match chunks.last() {
            Some(StreamChunk::Finish {
                reason: FinishReason::Error { failure },
                ..
            }) => {
                assert_eq!(failure.message, "boom");
                assert_eq!(failure.code, "SERVER");
            }
            other => panic!("expected error finish, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn aborted_signal_maps_to_aborted_finish() {
        let adapter: Arc<dyn LlmAdapter> = Arc::new(MockAdapter::new(vec![MockScript::Hang]));
        let mut runtime = LlmRuntime::new();
        runtime.register_adapter("mock", adapter);
        let options = options("mock");
        options.signal.abort();
        let chunks = collect(&runtime, options).await;
        match chunks.last() {
            Some(StreamChunk::Finish {
                reason: FinishReason::Aborted { failure },
                ..
            }) => {
                assert_eq!(failure.code, "ABORTED");
            }
            other => panic!("expected aborted finish, got {other:?}"),
        }
    }
}
