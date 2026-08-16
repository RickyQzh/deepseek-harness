# dsh-agent

English | [中文](README.zh.md)

Live `LoopAgent` registry for the DeepSeek Harness Rust host: `create`, `followup`, `run_until_idle`, and `status` keyed by session id.

The registry holds each agent behind `tokio::sync::Mutex<LoopAgent>` because `run_until_idle(&mut self)` cannot overlap `cancel`. `whenIdle` is `run_until_idle` then Idle. Live events use `Session::set_append_sink`.

YAML name `@deepseek-ai/dsh-agent` injects `llm`, `tools`, and `systemPrompt` and provides `agents`. The plugin stores the same kernel `Arc<Mutex<LlmRuntime>>` and `Arc<Mutex<ToolRuntime>>`; sibling plugins that `register_adapter` or `register` on those mutexes remain visible to `AgentRegistry::list_providers`. `register_spine_plugins` registers that plugin together with credentials, llm (mock, replay, and DeepSeek), tools, system-prompt, and the JSONL session store.

## Known Limitations and Deferred Work

- Persist flush, CLI, JSON-RPC dispatch, and execution plugins (subprocess, fs, shell, tool-fs, tool-bash) are later Phase 5 work.
