//! Kernel plugin `@deepseek-ai/dsh-web-search-deepseek`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_WEB_SEARCH_DEEPSEEK, PluginRegistry, PluginSetup};
use dsh_credentials::{CredentialProvider, LayeredCredentials, credential_ref};
use dsh_kernel::{Context, KernelError};
use dsh_web::{WebError, WebRuntime};
use serde_json::Value;

use crate::{
    DEEPSEEK_DEFAULT_API_VERSION, DEEPSEEK_DEFAULT_MAX_TOKENS, DEEPSEEK_DEFAULT_MAX_USES,
    DEEPSEEK_DEFAULT_MODEL, DeepSeekSearchProvider, DeepSeekSearchProviderOptions,
    resolve_base_url, search_base_url_from_env,
};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Validated plugin configuration.
#[derive(Clone, Debug)]
pub struct DeepSeekSearchConfig {
    /// Literal API key; non-empty wins over credential resolve.
    pub api_key: Option<String>,
    /// Credential reference resolved for each search.
    pub api_key_env: String,
    /// Optional Messages base override.
    pub base_url: Option<String>,
    /// Anthropic-format model name.
    pub model: String,
    /// `anthropic-version` header value.
    pub api_version: String,
    /// Messages `max_tokens`.
    pub max_tokens: u32,
    /// Native `web_search` `max_uses`.
    pub max_uses: u32,
}

impl Default for DeepSeekSearchConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            api_key_env: "DEEPSEEK_API_KEY".into(),
            base_url: None,
            model: DEEPSEEK_DEFAULT_MODEL.into(),
            api_version: DEEPSEEK_DEFAULT_API_VERSION.into(),
            max_tokens: DEEPSEEK_DEFAULT_MAX_TOKENS,
            max_uses: DEEPSEEK_DEFAULT_MAX_USES,
        }
    }
}

const CONFIG_KEYS: &[&str] = &[
    "apiKey",
    "apiKeyEnv",
    "baseURL",
    "model",
    "apiVersion",
    "maxTokens",
    "maxUses",
];

/// Register the DeepSeek search provider on `web`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved = resolve_config(&config).map_err(|error| setup_err(error.to_string()))?;
            let web = ctx.inject::<WebRuntime>("web").await?;
            let ctx_for_options = ctx.clone();
            let provider = DeepSeekSearchProvider::new(move || {
                options_from_context(&ctx_for_options, &resolved)
            });
            web.register_search_provider(Arc::new(provider))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_WEB_SEARCH_DEEPSEEK, setup);
}

fn options_from_context(
    ctx: &Context,
    config: &DeepSeekSearchConfig,
) -> DeepSeekSearchProviderOptions {
    let cred_name = config.api_key_env.clone();
    let credentials = ctx.get::<LayeredCredentials>("credentials");
    DeepSeekSearchProviderOptions {
        api_key: config
            .api_key
            .as_ref()
            .filter(|key| !key.is_empty())
            .cloned(),
        resolve_api_key: Some(Arc::new(move || {
            resolve_credential(credentials.as_deref(), &cred_name)
        })),
        api_key_env: config.api_key_env.clone(),
        base_url: resolve_base_url(
            config.base_url.as_deref(),
            search_base_url_from_env().as_deref(),
        ),
        model: config.model.clone(),
        api_version: config.api_version.clone(),
        max_tokens: config.max_tokens,
        max_uses: config.max_uses,
    }
}

fn resolve_credential(
    credentials: Option<&LayeredCredentials>,
    name: &str,
) -> Result<Option<String>, WebError> {
    if let Some(credentials) = credentials {
        let cred_ref = credential_ref(name).map_err(|error| {
            WebError::new(
                format!("DeepSeek search credential resolution failed: {error}"),
                dsh_web::WEB_PROVIDER_ERROR,
            )
        })?;
        match credentials.resolve(&cred_ref) {
            Ok(Some(resolved)) => Ok(Some(resolved.value)),
            Ok(None) => Ok(None),
            Err(error) => Err(WebError::new(
                format!("DeepSeek search credential resolution failed: {error}"),
                dsh_web::WEB_PROVIDER_ERROR,
            )),
        }
    } else {
        match std::env::var(name) {
            Ok(value) if !value.is_empty() => Ok(Some(value)),
            _ => Ok(None),
        }
    }
}

fn resolve_config(value: &Value) -> Result<DeepSeekSearchConfig, String> {
    match value {
        Value::Null => {
            credential_ref("DEEPSEEK_API_KEY").map_err(|error| error.to_string())?;
            Ok(DeepSeekSearchConfig::default())
        }
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("DeepSeekSearchConfig: unknown key \"{key}\""));
                }
            }
            let mut config = DeepSeekSearchConfig::default();
            if let Some(key) = optional_string(map.get("apiKey"), "apiKey")? {
                config.api_key = Some(key);
            }
            if let Some(env) = optional_string(map.get("apiKeyEnv"), "apiKeyEnv")? {
                if env.is_empty() {
                    return Err("DeepSeekSearchConfig.apiKeyEnv must be a non-empty string".into());
                }
                credential_ref(&env).map_err(|error| error.to_string())?;
                config.api_key_env = env;
            }
            if let Some(url) = optional_string(map.get("baseURL"), "baseURL")? {
                config.base_url = Some(url);
            }
            if let Some(model) = optional_string(map.get("model"), "model")? {
                if model.is_empty() {
                    return Err("DeepSeekSearchConfig.model must be a non-empty string".into());
                }
                config.model = model;
            }
            if let Some(version) = optional_string(map.get("apiVersion"), "apiVersion")? {
                if version.is_empty() {
                    return Err("DeepSeekSearchConfig.apiVersion must be a non-empty string".into());
                }
                config.api_version = version;
            }
            if let Some(tokens) = optional_positive_u32(map.get("maxTokens"), "maxTokens")? {
                config.max_tokens = tokens;
            }
            if let Some(uses) = optional_positive_u32(map.get("maxUses"), "maxUses")? {
                config.max_uses = uses;
            }
            Ok(config)
        }
        _ => Err("DeepSeekSearchConfig: config must be an object".into()),
    }
}

fn optional_string(value: Option<&Value>, key: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("DeepSeekSearchConfig.{key} must be a string")),
    }
}

fn optional_positive_u32(value: Option<&Value>, key: &str) -> Result<Option<u32>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(item) => {
            let invalid = || format!("DeepSeekSearchConfig.{key} must be a positive integer");
            let Some(number) = item.as_u64() else {
                return Err(invalid());
            };
            if number == 0 {
                return Err(invalid());
            }
            u32::try_from(number).map(Some).map_err(|_| invalid())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::DEEPSEEK_PROVIDER_ID;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_web::WebRuntime;
    use std::sync::Arc;

    #[tokio::test]
    async fn registers_on_web() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_web::plugin::register(&mut registry);
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-web'\n- name: '@deepseek-ai/dsh-web-search-deepseek'\n  config:\n    apiKey: ds-key\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot deepseek search");
        let web = ctx.inject::<WebRuntime>("web").await.expect("web");
        let err = web
            .register_search_provider(Arc::new(crate::DeepSeekSearchProvider::new(|| {
                panic!("duplicate")
            })))
            .unwrap_err();
        assert!(err.to_string().contains(DEEPSEEK_PROVIDER_ID));
    }
}
