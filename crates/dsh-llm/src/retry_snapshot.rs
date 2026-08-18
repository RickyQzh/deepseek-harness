//! Deterministic adapter: first stream `RATE_LIMIT` (HTTP 429), then text `RETRY_OK`.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_RETRY_SNAPSHOT_BACKEND, PluginRegistry, PluginSetup};
use dsh_session::StreamChunk;
use futures::Stream;
use serde_json::Value;

use crate::LlmRuntime;
use crate::adapter::LlmAdapter;
use crate::error::LlmError;
use crate::mock::text_response;
use crate::retry::{ResolvedRetryPolicy, RetryBackoff, RetryMode};
use crate::types::GenerateOptions;

/// Snapshot backend used by the headless provider-retry inspect.
pub struct FailThenOk {
    calls: Mutex<u32>,
    first_messages: Mutex<Option<String>>,
    policy: ResolvedRetryPolicy,
}

impl FailThenOk {
    /// One `RATE_LIMIT` with `status: 429`, then assembled text `RETRY_OK`.
    #[must_use]
    pub fn rate_limit() -> Self {
        Self {
            calls: Mutex::new(0),
            first_messages: Mutex::new(None),
            policy: ResolvedRetryPolicy {
                mode: RetryMode::Normal,
                max_retries: 1,
                retryable_codes: vec!["RATE_LIMIT".into()],
                backoff: RetryBackoff {
                    initial_delay_ms: 1,
                    max_delay_ms: 1,
                    jitter_ratio: 0.0,
                },
            },
        }
    }
}

impl LlmAdapter for FailThenOk {
    fn retry_policy(&self) -> ResolvedRetryPolicy {
        self.policy.clone()
    }

    fn stream(
        &self,
        options: GenerateOptions,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LlmError>> + Send + '_>> {
        let messages = serde_json::to_string(&options.messages).expect("messages json");
        let n = {
            let mut calls = self.calls.lock().expect("fail-then-ok calls");
            *calls += 1;
            *calls
        };
        if n == 1 {
            *self.first_messages.lock().expect("fail-then-ok messages") = Some(messages);
            return Box::pin(futures::stream::iter([Err(LlmError::new(
                "snapshot transient failure",
                "RATE_LIMIT",
            )
            .with_status(429))]));
        }
        if n == 2 {
            let first = self.first_messages.lock().expect("fail-then-ok messages");
            if first.as_ref() != Some(&messages) {
                return Box::pin(futures::stream::iter([Err(LlmError::new(
                    "retry snapshot changed the model-visible messages",
                    "UNKNOWN",
                ))]));
            }
        }
        Box::pin(futures::stream::iter(
            text_response("RETRY_OK").into_iter().map(Ok),
        ))
    }
}

/// Register YAML `retry-snapshot-backend` on provider route `deepseek-official`.
pub fn register_retry_snapshot_backend(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config: Value| {
        Box::pin(async move {
            let llm = ctx.inject::<Mutex<LlmRuntime>>("llm").await?;
            llm.lock()
                .expect("llm")
                .register_adapter("deepseek-official", Arc::new(FailThenOk::rate_limit()));
            Ok(())
        })
    });
    registry.register(PLUGIN_RETRY_SNAPSHOT_BACKEND, setup);
}
