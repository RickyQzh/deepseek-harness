# Agent Note: Bounded wait for LLM providers after headless-runner spawn

Status: implemented

[English](2026-08-16-rust-wait-llm-providers.md) | 中文

## 问题

`headless-runner` 的 setup 会 `tokio::spawn` `run` 然后返回，以便后续 YAML 插件可以挂载。`run` 在 `create` 或 `resume` 之前读取 `AgentRegistry::list_providers()`。兄弟适配器插件可能仍在 `register_adapter` 中，因此第一次轮询可以为空，进程以退出码 1 退出，并报告 `plugin setup failed: no LLM provider registered`。

`boot_yaml` 会启动每一行的插件 fiber，然后 `await_ready`。`@deepseek-ai/dsh-tool-subagent` 注入 `subagents` 并调用 `get_provider` 时，`@deepseek-ai/dsh-subagent-spawn-in-process` 可能仍在 `register_provider` 中，于是加载失败并报告 `tool-subagent: unknown provider "spawn"`。

## 决策

在 `create` 或 `resume` 之前，`run_inner` 调用私有的 `wait_until_providers`，它轮询 `list_providers()` 直到列表非空，或 64 次 yield 加 1ms 的尝试用尽。有界次数用尽后仍为空时，仍然失败并报告 `no LLM provider registered`。`AgentRegistry` 不增加等待方法。

`@deepseek-ai/dsh-tool-subagent` 的 setup 在 `register_delegate_tool` 之前用同一有界次数调用私有的 `wait_until_named_provider`。有界次数用尽后仍为空时，仍然失败并报告 `tool-subagent: unknown provider "{name}"`。JSON-RPC 的 `initialize` 不轮询 `list_providers`：该 bin 在 `boot_yaml` 返回之后才调用 `serve`。

## 测试

`wait_until_providers_sees_ids_after_empty_polls` 在先出现空轮询后返回后来的 id。`wait_until_providers_fails_when_empty_after_bound` 在每次轮询都为空时失败，并报告 `no LLM provider registered`。`wait_until_sees_ready_after_empty_polls` 与 `wait_until_named_provider_fails_when_missing_after_bound` 固定委托工具的等待。

## 考虑过的替代方案

**等 `run` 结束后才从 `headless-runner` setup 返回。** 不予采用：setup 必须返回，剩余 YAML 插件才能挂载；这正是 spawn `run` 的原因。

**严格按 YAML 列表顺序挂载各行。** 不予采用：`ctx.plugin` 加 `await_ready` 已是现有 boot；再做一遍顺序挂载会改变所有插件的重叠假设。

**永远等到出现一个 provider。** 不予采用：缺失的适配器或 subagent 提供方仍然必须大声失败。

**新增 `AgentRegistry::wait_for_providers` 或 `SubagentRuntime::wait_for_provider`。** 不予采用：辅助函数留在 `dsh-headless` 与 `dsh-tool-subagent` 内部。

**固定睡眠一次。** 不予采用：单次睡眠在负载下仍会输掉，并拖慢注册已经可见的路径。

## 后果

从未注册 LLM 适配器或指定 subagent 提供方的宿主会等待直到有界次数用尽（约 64ms），然后再报告同一条 setup 错误。成功 boot 之后，当指定的 LLM provider 不存在时，JSON-RPC `initialize` 仍然立即失败。

## 相关

JSON-RPC 在 `boot_yaml` 之后才 `serve`，因此 `initialize` 能看到兄弟插件的 `register_adapter`（[Rust SDK JSON-RPC server](2026-08-16-rust-sdk-jsonrpc-server.md)）。Headless 的消费者说明见 [dsh-headless](../../../../crates/dsh-headless/README.md)。委托工具加载见 [dsh-tool-subagent](../../../../crates/dsh-tool-subagent/README.md)。
