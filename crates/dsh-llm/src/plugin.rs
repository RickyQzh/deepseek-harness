//! Kernel plugins `@deepseek-ai/dsh-llm` and `@deepseek-ai/dsh-llm-mock`.

use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_LLM, PLUGIN_LLM_MOCK, PluginRegistry, PluginSetup};
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
