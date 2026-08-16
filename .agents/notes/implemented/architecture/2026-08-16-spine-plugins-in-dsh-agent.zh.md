# Agent Note: Spine YAML plugins register from dsh-agent, not dsh-boot

Status: implemented

[English](2026-08-16-spine-plugins-in-dsh-agent.md) | 中文

## 问题

`PluginRegistry` 与封闭的 YAML `name` 常量位于 `dsh-boot`。每个产品插件的 `register` 都需要该类型。若 `dsh-boot` 再依赖这些产品 crate 来调用 `register`，Cargo 会形成环（`dsh-boot` → 产品 crate → `dsh-boot`）。

## 决策

产品 crate 依赖 `dsh-boot` 与 `dsh-kernel`，并导出 `plugin::register`（llm 另外导出 `plugin::register_llm`、`plugin::register_mock` 与 `replay::register`）。`dsh_agent::register_spine_plugins` 是 spine 组合函数，负责注册 credentials、llm（mock、replay、DeepSeek）、tools、system-prompt、agent 以及 JSONL 会话存储。`dsh-boot` 不依赖产品 crate；其测试只保留 probe 插件。

YAML 名称仍是 `dsh-boot` 中的封闭常量（`@deepseek-ai/dsh-credentials`、`@deepseek-ai/dsh-llm`、`@deepseek-ai/dsh-llm-deepseek`、`@deepseek-ai/dsh-llm-mock`、`@deepseek-ai/dsh-llm-replay`、`@deepseek-ai/dsh-tools`、`@deepseek-ai/dsh-system-prompt`、`@deepseek-ai/dsh-agent`、`@deepseek-ai/dsh-session-persistence-jsonl`）。setup 在插件 fiber 内用 `ctx.inject` 等待；`ctx.plugin` 不接收 inject 名称。对已经提供的服务调用 `inject` 会在 slot 存在时立即返回；它不会等待仍在该互斥锁上执行 `register_adapter` 或 `register` 的兄弟插件。因此 `@deepseek-ai/dsh-agent` 把注入的 `llm` 与 `tools` 互斥锁 Arc 存进 `AgentRegistry`，而不是在 setup 时克隆内部 map。

DeepSeek 插件把 `CredentialRef` 存在 `DeepSeekConnectionOptions` 上，并在每次 `stream` 通过 `LayeredCredentials` 解析密钥；适配器从不存储原始密钥。Replay 在已配置的 provider id 之间共享同一个 `Arc<ReplayAdapter>`，并从 `DSH_SNAPSHOT_FILE` 弹出 `assistant/chunk` 轮次。

## 备选方案

**在 `dsh-boot` 中放置 `register_spine_plugins` 并依赖产品 crate。** 否决：这正是本笔记要防止的环。

**在 `dsh-cli` 与 `dsh-sdk-jsonrpc-server` 中各写一份 `register_spine_plugins`。** 否决：两份副本会漂移；两个 bin 都调用 `dsh_agent::register_spine_plugins`。

**抽出新的 `dsh-product` crate。** 本阶段否决：任务禁止额外 crate，且 `dsh-agent` 已经依赖 llm、tools 与 system-prompt。

**产品 crate 不依赖 `dsh-boot`，只把 YAML 名称写成字符串字面量。** 否决：`register` 仍需要 `&mut PluginRegistry`，而该类型定义在 `dsh-boot`。

## 影响

Headless 与 JSON-RPC bin 调用 `dsh_agent::register_spine_plugins`，而不是导入每一个 spine crate。新增 spine 插件意味着在其 crate 中写 `register`，并在 `dsh-agent` 中增加一次调用。该函数不注册执行插件（subprocess、fs、shell、tool-fs、tool-bash）；这些名称由兄弟函数 `register_execution_plugins` 注册（[Execution YAML plugins](2026-08-16-execution-plugins-in-dsh-agent.md)）。

`cargo test -p dsh-agent --offline spine` 会启动一份 mock YAML 列表，并断言 `credentials`、`llm`、`tools`、`systemPrompt`、`agents` 与 `sessions`，以及 `AgentRegistry::list_providers` 包含 `mock`。未知 YAML 名称仍然会作为加载错误失败。该 YAML 列表不挂载 `@deepseek-ai/dsh-llm-replay` 或 `@deepseek-ai/dsh-llm-deepseek`。

## 相关

程序级重写提案见 [Rewrite core and backend in Rust](../../proposed/architecture/2026-08-14-rust-rewrite.md)。
