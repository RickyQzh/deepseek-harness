//! `@deepseek-ai/dsh-llm-replay`: serve `assistant/chunk` runs from `DSH_SNAPSHOT_FILE`.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_LLM_REPLAY, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_session::{LogEvent, SessionEvent, StreamChunk};
use dsh_session_persist::decode_session_log;
use futures::Stream;
use serde_json::Value;

use crate::types::{LlmModelContext, LlmResolvedModelInfo};
use crate::{GenerateOptions, LlmAdapter, LlmError, LlmRuntime};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

struct ReplayAdapter {
    scripts: Mutex<Vec<Vec<StreamChunk>>>,
    context_windows: HashMap<(String, String), u64>,
}

impl LlmAdapter for ReplayAdapter {
    fn resolve_model(&self, provider: &str, model: &str) -> LlmResolvedModelInfo {
        let context = self
            .context_windows
            .get(&(provider.to_string(), model.to_string()))
            .copied()
            .map(|context_window| LlmModelContext { context_window });
        LlmResolvedModelInfo {
            provider: provider.into(),
            id: model.into(),
            name: model.into(),
            context,
            default_max_tokens: None,
            reasoning: None,
        }
    }

    fn stream(
        &self,
        _options: GenerateOptions,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LlmError>> + Send + '_>> {
        let next = self.scripts.lock().expect("replay").pop();
        match next {
            Some(chunks) => Box::pin(futures::stream::iter(chunks.into_iter().map(Ok))),
            None => Box::pin(futures::stream::iter([Err(LlmError::new(
                "llm-replay: script exhausted",
                "UNKNOWN",
            ))])),
        }
    }
}

fn scripts_from_file(path: &str) -> Result<Vec<Vec<StreamChunk>>, KernelError> {
    let text = std::fs::read_to_string(path).map_err(|error| setup_err(error.to_string()))?;
    let (_header, events) =
        decode_session_log(&text).map_err(|error| setup_err(error.to_string()))?;
    let mut runs: Vec<Vec<StreamChunk>> = Vec::new();
    let mut current: Vec<StreamChunk> = Vec::new();
    let mut last_key: Option<(u64, u64)> = None;
    for event in events {
        let LogEvent::Known(SessionEvent::AssistantChunk { data, .. }) = event else {
            continue;
        };
        let key = (data.turn, data.step);
        if last_key.is_some() && last_key != Some(key) && !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
        last_key = Some(key);
        current.push(data.chunk);
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs.reverse();
    Ok(runs)
}

fn context_windows_from_providers(providers: &[Value]) -> HashMap<(String, String), u64> {
    let mut windows = HashMap::new();
    for provider in providers {
        let provider_id = provider
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("deepseek-official");
        let Some(models) = provider.get("models").and_then(Value::as_array) else {
            continue;
        };
        for model in models {
            let Some(model_id) = model.get("id").and_then(Value::as_str) else {
                continue;
            };
            let Some(window) = model
                .get("contextWindow")
                .and_then(|value| value.as_u64().or_else(|| value.as_i64().map(|n| n as u64)))
            else {
                continue;
            };
            windows.insert((provider_id.to_string(), model_id.to_string()), window);
        }
    }
    windows
}

/// Register the replay adapter. Config `providers` is an array of `{id: string}`; default id `deepseek-official`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let llm = ctx.inject::<Mutex<LlmRuntime>>("llm").await?;
            let path = std::env::var("DSH_SNAPSHOT_FILE").map_err(|_| {
                setup_err("DSH_SNAPSHOT_FILE is required for @deepseek-ai/dsh-llm-replay")
            })?;
            let scripts = scripts_from_file(&path)?;
            let providers = config
                .get("providers")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_else(|| vec![json_provider("deepseek-official")]);
            let context_windows = context_windows_from_providers(&providers);
            let adapter: Arc<dyn LlmAdapter> = Arc::new(ReplayAdapter {
                scripts: Mutex::new(scripts),
                context_windows,
            });
            for provider in providers {
                let id = provider
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("deepseek-official")
                    .to_string();
                llm.lock()
                    .expect("llm")
                    .register_adapter(id, Arc::clone(&adapter));
            }
            Ok(())
        })
    });
    registry.register(PLUGIN_LLM_REPLAY, setup);
}

fn json_provider(id: &str) -> Value {
    serde_json::json!({"id": id})
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::LlmRuntime;
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;
    use dsh_session::LlmCallConfig;
    use std::sync::Mutex;

    fn snapshot_header() -> String {
        "{\"type\":\"session\",\"version\":0,\"id\":\"replay-ctx\",\"createdAt\":0,\"delegationDepth\":0}\n"
            .to_string()
    }

    #[tokio::test]
    async fn replay_resolve_model_reads_context_window() {
        let path = std::env::temp_dir().join(format!(
            "dsh-replay-ctx-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::write(&path, snapshot_header()).expect("write snapshot");
        let previous = std::env::var("DSH_SNAPSHOT_FILE").ok();
        unsafe {
            std::env::set_var("DSH_SNAPSHOT_FILE", &path);
        }
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        crate::plugin::register_llm(&mut registry);
        register(&mut registry);
        let yaml = "\
- name: '@deepseek-ai/dsh-llm'
- name: '@deepseek-ai/dsh-llm-replay'
  config:
    providers:
      - id: deepseek-official
        models:
          - id: deepseek-v4-flash
            contextWindow: 128000
";
        assert!(!yaml.contains("!!js"));
        let result = boot_yaml(&ctx, yaml, &[], &registry, &process_interpolate_env()).await;
        match previous {
            Some(value) => unsafe { std::env::set_var("DSH_SNAPSHOT_FILE", value) },
            None => unsafe { std::env::remove_var("DSH_SNAPSHOT_FILE") },
        }
        let _ = std::fs::remove_file(&path);
        result.expect("boot replay");
        let llm = ctx.get::<Mutex<LlmRuntime>>("llm").expect("llm");
        let prepared = llm
            .lock()
            .expect("llm")
            .prepare_call(&LlmCallConfig {
                provider: "deepseek-official".into(),
                model: "deepseek-v4-flash".into(),
                reasoning_effort: None,
                temperature: None,
                max_tokens: None,
                stop: None,
            })
            .expect("prepare");
        assert_eq!(
            prepared.context,
            Some(crate::LlmModelContext {
                context_window: 128000
            })
        );
    }
}
