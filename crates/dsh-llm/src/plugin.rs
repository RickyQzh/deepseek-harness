//! Kernel plugins `@deepseek-ai/dsh-llm`, `@deepseek-ai/dsh-llm-mock`, and `@deepseek-ai/dsh-llm-retry`.

use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_LLM, PLUGIN_LLM_MOCK, PLUGIN_LLM_RETRY, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::{LlmRuntime, MockAdapter, MockScript, text_response};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register the empty LLM runtime plugin.
pub fn register_llm(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config: Value| {
        Box::pin(async move {
            ctx.provide("llm", Mutex::new(LlmRuntime::new()))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_LLM, setup);
}

/// Register a scripted mock adapter. Config: `provider` (default `mock`), `text` (default `ok`).
pub fn register_mock(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let llm = ctx.inject::<Mutex<LlmRuntime>>("llm").await?;
            let provider = config
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("mock")
                .to_string();
            let text = config
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("ok")
                .to_string();
            let adapter = Arc::new(MockAdapter::new(vec![MockScript::Chunks(text_response(
                &text,
            ))]));
            llm.lock().expect("llm").register_adapter(provider, adapter);
            Ok(())
        })
    });
    registry.register(PLUGIN_LLM_MOCK, setup);
}

/// Register provider-routed retry. Config must be empty; providers own `retryPolicy`.
pub fn register_retry(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            reject_retry_config(&config)?;
            crate::retry::install(&ctx);
            Ok(())
        })
    });
    registry.register(PLUGIN_LLM_RETRY, setup);
}

fn reject_retry_config(config: &Value) -> Result<(), KernelError> {
    let Some(obj) = config.as_object() else {
        return Ok(());
    };
    if obj.contains_key("retryPolicy") {
        return Err(setup_err(
            "llm-retry: retryPolicy belongs under each provider configuration",
        ));
    }
    let Some(key) = obj.keys().next() else {
        return Ok(());
    };
    Err(setup_err(format!("llm-retry: unknown key \"{key}\"")))
}
