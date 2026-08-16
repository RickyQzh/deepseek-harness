# dsh-agent

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的实时 `LoopAgent` 注册表：按会话 id 提供 `create`、`resume`、`followup`、`run_until_idle`、`when_idle`、`status` 与 `list`。

注册表将每个 agent（智能体）存为 `AgentInner { driver: tokio::sync::Mutex<()>, state: std::sync::Mutex<LoopAgent> }`。`run_until_idle` 取得 `driver`，仅在同步会话变更期间锁定 `state`，并在 LLM 流与工具 body 的 `.await` 上释放，以便并发的 `AgentHandle::followup` 能拼接 inbox。`followup` / `steer` / `inject` 只锁定 `state`。`cancel` 先中止标志再取得 `driver`。`whenIdle` 是 `run_until_idle` 然后 Idle。`list` 以未指定顺序返回实时句柄。`on_session_create` 在 `LoopAgent::new` 之前运行。`resume` 用已加载的 `Session` 重建而不是 `Session::new`，并在该会话 id 已注册时返回现有句柄。实时事件使用 `Session::set_append_sink`。`create` 把 kernel 的 `Context` 以及同一把 `llm` / `tools` 互斥锁 Arc 存入每个 `LoopAgent`。`resume_with_tools` 接受一把可选的子会话本地 `ToolRuntime` 互斥锁；`None` 使用注册表共享的 tools。可继续子 agent 传入克隆，以便 `report` 能注册且不会出现在父级运行时上。

YAML 名称为 `@deepseek-ai/dsh-agent`，注入 `llm`、`tools` 和 `systemPrompt`，并提供 `agents`。该插件保存内核提供的同一把 `Arc<Mutex<LlmRuntime>>` 与 `Arc<Mutex<ToolRuntime>>`；兄弟插件在这些互斥锁上的 `register_adapter` 或 `register` 对 `AgentRegistry::list_providers` 仍然可见。`register_spine_plugins` 将该插件与 credentials、llm（含 mock、replay 与 DeepSeek）、tools、system-prompt 以及 JSONL 会话存储一并注册。`register_execution_plugins` 挂载 subprocess、fs、shell、tool-fs 与 tool-bash。

## 已知限制与暂缓事项

- 本 crate 不持久化会话；宿主在 `when_idle` 之后对 `list()` 返回的每个句柄执行 flush。
