# dsh-agent

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的实时 `LoopAgent` 注册表：按会话 id 提供 `create`、`followup`、`run_until_idle` 与 `status`。

注册表用 `tokio::sync::Mutex<LoopAgent>` 持有每个 agent（智能体），因为 `run_until_idle(&mut self)` 不能与 `cancel` 重叠。`whenIdle` 是 `run_until_idle` 然后 Idle。实时事件使用 `Session::set_append_sink`。

## 已知限制与暂缓事项

- 持久化 flush、CLI（命令行界面）、JSON-RPC 分发以及 kernel 插件包装属于后续第 5 阶段工作。本 crate 不加载插件。
