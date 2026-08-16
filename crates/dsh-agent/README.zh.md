# dsh-agent

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的实时 `LoopAgent` 注册表：按会话 id 提供 `create`、`followup`、`run_until_idle` 与 `status`。

注册表用 `tokio::sync::Mutex<LoopAgent>` 持有每个 agent（智能体），因为 `run_until_idle(&mut self)` 不能与 `cancel` 重叠。`whenIdle` 是 `run_until_idle` 然后 Idle。实时事件使用 `Session::set_append_sink`。

YAML 名称为 `@deepseek-ai/dsh-agent`，注入 `llm`、`tools` 和 `systemPrompt`，并提供 `agents`。`register_spine_plugins` 将该插件与 credentials、llm（含 mock、replay 与 DeepSeek）、tools、system-prompt 以及 JSONL 会话存储一并注册。

## 已知限制与暂缓事项

- 持久化 flush、CLI（命令行界面）、JSON-RPC 分发以及执行插件（subprocess、fs、shell、tool-fs、tool-bash）属于后续第 5 阶段工作。
