//! `@deepseek-ai/dsh-llm-replay`: serve `assistant/chunk` runs from `DSH_SNAPSHOT_FILE`.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use dsh_boot::{PLUGIN_LLM_REPLAY, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use dsh_session::{LogEvent, SessionEvent, StreamChunk};
use dsh_session_persist::decode_session_log;
use futures::Stream;
use serde_json::Value;

use crate::{GenerateOptions, LlmAdapter, LlmError, LlmRuntime};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

struct ReplayAdapter {
    scripts: Mutex<Vec<Vec<StreamChunk>>>,
}

impl LlmAdapter for ReplayAdapter {
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
            let adapter: Arc<dyn LlmAdapter> = Arc::new(ReplayAdapter {
                scripts: Mutex::new(scripts),
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
