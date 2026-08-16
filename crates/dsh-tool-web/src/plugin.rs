//! Kernel plugin `@deepseek-ai/dsh-tool-web`.

use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_TOOL_WEB, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_tools::ToolRuntime;
use dsh_web::WebRuntime;
use serde_json::Value;

use crate::search::{WEB_SEARCH_MAX_RESULTS, register_web_search_tool};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Default cooperative tool-call timeout budget (ms) for `web_search`.
pub const DEFAULT_SEARCH_TIMEOUT_MS: u64 = 60_000;

/// Validated `dsh-tool-web` configuration. `fetch` defaults to false in this phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolWebConfig {
    /// Register `web_search`.
    pub search: bool,
    /// Upper bound on sources returned by one `web_search` call.
    pub search_max_results: u32,
    /// Cooperative timeout budget in milliseconds (stored for timeout-policy; not on `ToolDefinition` yet).
    pub search_timeout_ms: u64,
}

impl Default for ToolWebConfig {
    fn default() -> Self {
        Self {
            search: true,
            search_max_results: WEB_SEARCH_MAX_RESULTS,
            search_timeout_ms: DEFAULT_SEARCH_TIMEOUT_MS,
        }
    }
}

const CONFIG_KEYS: &[&str] = &["search", "fetch", "searchMaxResults", "searchTimeoutMs"];

/// Register `web_search` on `tools`. `fetch: true` fails load before inject.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved = resolve_config(&config).map_err(|error| setup_err(error.to_string()))?;
            if !resolved.search {
                return Ok(());
            }
            let tools = ctx.inject::<Mutex<ToolRuntime>>("tools").await?;
            let web = ctx.inject::<WebRuntime>("web").await?;
            register_web_search_tool(
                &mut tools
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                web,
                resolved.search_max_results,
            );
            Ok(())
        })
    });
    registry.register(PLUGIN_TOOL_WEB, setup);
}

fn resolve_config(value: &Value) -> Result<ToolWebConfig, String> {
    match value {
        Value::Null => Ok(ToolWebConfig::default()),
        Value::Object(map) => {
            match map.get("fetch") {
                Some(Value::Bool(true)) => {
                    return Err("web_fetch remains off in Phase 6".into());
                }
                None | Some(Value::Bool(false)) | Some(Value::Null) => {}
                Some(_) => return Err("ToolWebConfig.fetch must be a boolean".into()),
            }
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(format!("ToolWebConfig: unknown key \"{key}\""));
                }
            }
            let mut config = ToolWebConfig::default();
            if let Some(flag) = optional_bool(map.get("search"), "search")? {
                config.search = flag;
            }
            if let Some(max) =
                optional_positive_u32(map.get("searchMaxResults"), "searchMaxResults")?
            {
                config.search_max_results = max;
            }
            if let Some(timeout) =
                optional_positive_u64(map.get("searchTimeoutMs"), "searchTimeoutMs")?
            {
                config.search_timeout_ms = timeout;
            }
            Ok(config)
        }
        _ => Err("ToolWebConfig: config must be an object".into()),
    }
}

fn optional_bool(value: Option<&Value>, key: &str) -> Result<Option<bool>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(format!("ToolWebConfig.{key} must be a boolean")),
    }
}

fn optional_positive_u32(value: Option<&Value>, key: &str) -> Result<Option<u32>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(item) => {
            let invalid = || format!("tool-web: {key} must be a positive integer");
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

fn optional_positive_u64(value: Option<&Value>, key: &str) -> Result<Option<u64>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(item) => {
            let invalid = || format!("tool-web: {key} must be a positive integer");
            let Some(number) = item.as_u64() else {
                return Err(invalid());
            };
            if number == 0 {
                return Err(invalid());
            }
            Ok(Some(number))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::WEB_SEARCH_MAX_RESULTS;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_session::CallId;
    use dsh_tools::{AbortFlag, ToolExecutionInput, ToolExecutionResult, ToolRuntime};
    use dsh_web::{
        WebRuntime, WebSearchProvider, WebSearchRequest, WebSearchResult, WebSearchSource,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    async fn boot_tool_web_yaml(config_yaml: &str) -> Result<(), dsh_boot::BootError> {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        let yaml = format!("- name: '@deepseek-ai/dsh-tool-web'\n  config:\n    {config_yaml}\n");
        boot_yaml(&ctx, &yaml, &[], &registry, &process_interpolate_env())
            .await
            .map(|_| ())
    }

    #[tokio::test]
    async fn fetch_true_fails_plugin_load() {
        let err = boot_tool_web_yaml("fetch: true").await.unwrap_err();
        assert!(err.to_string().contains("web_fetch remains off"));
    }

    struct FakeSearch;

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
            Box<
                dyn std::future::Future<Output = Result<WebSearchResult, dsh_web::WebError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                Ok(WebSearchResult {
                    content: None,
                    sources: vec![WebSearchSource {
                        url: "https://example.test".into(),
                        title: Some("Example".into()),
                        snippet: Some("hello".into()),
                        published_at: None,
                    }],
                    truncated: false,
                })
            })
        }
    }

    async fn boot_search_stack() -> (Arc<WebRuntime>, Arc<Mutex<ToolRuntime>>) {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        dsh_tools::plugin::register(&mut registry);
        dsh_web::plugin::register(&mut registry);
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-tools'\n- name: '@deepseek-ai/dsh-web'\n- name: '@deepseek-ai/dsh-tool-web'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot tool-web stack");
        let web = ctx.inject::<WebRuntime>("web").await.expect("web");
        web.register_search_provider(Arc::new(FakeSearch))
            .expect("fake search");
        let tools = ctx
            .inject::<Mutex<ToolRuntime>>("tools")
            .await
            .expect("tools");
        (web, tools)
    }

    #[tokio::test]
    async fn omitted_fetch_registers_web_search_only() {
        let (_web, tools) = boot_search_stack().await;
        let names = tools.lock().expect("tools").registered_names();
        assert_eq!(names, vec!["web_search".to_string()]);
        assert_eq!(WEB_SEARCH_MAX_RESULTS, 8);
    }

    #[tokio::test]
    async fn empty_query_is_a_tool_error_string() {
        let (_web, tools) = boot_search_stack().await;
        let result = tools
            .lock()
            .expect("tools")
            .execute(ToolExecutionInput {
                call_id: CallId::new("c1"),
                root_call_id: None,
                name: "web_search".into(),
                arguments: json!({ "query": "  " }),
                parent: None,
                signal: AbortFlag::new(),
            })
            .await;
        assert!(result.is_error());
        match result {
            ToolExecutionResult::Failure { error, .. } => {
                assert!(error.message.contains("query must be a non-empty string"));
            }
            ToolExecutionResult::Success { .. } => panic!("expected failure"),
        }
    }

    #[tokio::test]
    async fn web_search_formats_sources() {
        let (_web, tools) = boot_search_stack().await;
        let result = tools
            .lock()
            .expect("tools")
            .execute(ToolExecutionInput {
                call_id: CallId::new("c1"),
                root_call_id: None,
                name: "web_search".into(),
                arguments: json!({ "query": "rust" }),
                parent: None,
                signal: AbortFlag::new(),
            })
            .await;
        assert!(!result.is_error());
        let text = match result.content() {
            [dsh_session::ContentBlock::Text { text }] => text,
            other => panic!("unexpected content {other:?}"),
        };
        assert!(text.contains("[Example](https://example.test) — hello"));
        assert!(text.contains("Cite the relevant URLs"));
    }
}
