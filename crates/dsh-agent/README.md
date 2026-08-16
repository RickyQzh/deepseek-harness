# dsh-agent

English | [中文](README.zh.md)

Live `LoopAgent` registry for the DeepSeek Harness Rust host: `create`, `followup`, `run_until_idle`, and `status` keyed by session id.

The registry holds each agent as `AgentInner { driver: tokio::sync::Mutex<()>, state: std::sync::Mutex<LoopAgent> }`. `run_until_idle` acquires `driver` and locks `state` only around synchronous session mutations, releasing it across the LLM stream and tool-body `.await` so a concurrent `AgentHandle::followup` can splice the inbox. `followup` / `steer` / `inject` lock `state` only. `cancel` aborts the flag then acquires `driver`. `whenIdle` is `run_until_idle` then Idle. `on_session_create` runs before `LoopAgent::new`. Live events use `Session::set_append_sink`. `create` stores the kernel `Context` and the same `llm` / `tools` mutex Arcs on each `LoopAgent`.

YAML name `@deepseek-ai/dsh-agent` injects `llm`, `tools`, and `systemPrompt` and provides `agents`. The plugin stores the same kernel `Arc<Mutex<LlmRuntime>>` and `Arc<Mutex<ToolRuntime>>`; sibling plugins that `register_adapter` or `register` on those mutexes remain visible to `AgentRegistry::list_providers`. `register_spine_plugins` registers that plugin together with credentials, llm (mock, replay, and DeepSeek), tools, system-prompt, and the JSONL session store. `register_execution_plugins` mounts subprocess, fs, shell, tool-fs, and tool-bash.

## Known Limitations and Deferred Work

- Persist flush, CLI, and JSON-RPC dispatch are later Phase 5 work.
