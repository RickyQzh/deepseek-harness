//! Kernel plugin `@deepseek-ai/dsh-llm-deepseek`.

use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_LLM_DEEPSEEK, PluginRegistry, PluginSetup};
use dsh_credentials::{CredentialProvider, LayeredCredentials, credential_ref};
use dsh_kernel::KernelError;
use dsh_llm::{INVALID_CREDENTIAL_CODE, LlmError, LlmRuntime, assert_usable_api_key};
use serde_json::Value;

use crate::{
    DEFAULT_CONTEXT_WINDOW, DEFAULT_MAX_TOKENS, DEFAULT_STREAM_IDLE_TIMEOUT_MS, DeepSeekAdapter,
    DeepSeekConnectionOptions, RequestDefaults,
};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register DeepSeek on provider `deepseek-official`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let llm = ctx.inject::<Mutex<LlmRuntime>>("llm").await?;
            let credentials = ctx.inject::<LayeredCredentials>("credentials").await?;
            let api_key_env = config
                .get("apiKeyEnv")
                .and_then(Value::as_str)
                .unwrap_or("DEEPSEEK_API_KEY")
                .to_string();
            let cred_ref =
                credential_ref(&api_key_env).map_err(|error| setup_err(error.to_string()))?;
            let adapter = DeepSeekAdapter::new(
                {
                    let api_key_env = cred_ref.clone();
                    move || DeepSeekConnectionOptions {
                        base_url: std::env::var("DEEPSEEK_BASE_URL")
                            .unwrap_or_else(|_| "https://api.deepseek.com".into()),
                        api_key_env: api_key_env.clone(),
                        defaults: RequestDefaults::default(),
                        max_tokens: DEFAULT_MAX_TOKENS,
                        default_context_window: DEFAULT_CONTEXT_WINDOW,
                        stream_idle_timeout: std::time::Duration::from_millis(
                            DEFAULT_STREAM_IDLE_TIMEOUT_MS,
                        ),
                    }
                },
                {
                    let credentials = Arc::clone(&credentials);
                    move |options: &DeepSeekConnectionOptions| {
                        let resolved =
                            credentials.resolve(&options.api_key_env).map_err(|error| {
                                LlmError::new(error.to_string(), INVALID_CREDENTIAL_CODE)
                            })?;
                        let Some(resolved) = resolved else {
                            return Err(LlmError::new(
                                format!("missing credential {}", options.api_key_env.as_str()),
                                INVALID_CREDENTIAL_CODE,
                            ));
                        };
                        assert_usable_api_key(
                            &resolved.value,
                            "dsh-llm-deepseek",
                            options.api_key_env.as_str(),
                        )
                    }
                },
                "dsh-rust",
            );
            llm.lock()
                .expect("llm")
                .register_adapter("deepseek-official", Arc::new(adapter));
            Ok(())
        })
    });
    registry.register(PLUGIN_LLM_DEEPSEEK, setup);
}
