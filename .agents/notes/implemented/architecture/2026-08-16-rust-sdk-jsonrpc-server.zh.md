# Agent Note: Rust SDK JSON-RPC server and dsh-jsonrpc-agent bin

Status: implemented

[English](2026-08-16-rust-sdk-jsonrpc-server.md) | 中文

## 问题

Python 与 TypeScript SDK 把 harness 当作 stdio 子进程，通过 NDJSON JSON-RPC 2.0 驱动。 [Rust 重写](../../proposed/architecture/2026-08-14-rust-rewrite.md) 的第 5 阶段需要在 Rust 二进制上提供同一条线路，同时不把产品 crate 依赖放进 `dsh-boot`，不使用 JavaScript YAML 标签，也不把诊断混进 stdout。

## 决策

`dsh-sdk-jsonrpc-server` 是 SDK 服务器插件与 `dsh-jsonrpc-agent` bin。YAML 名称 `sdk-jsonrpc-server` 是 `dsh-boot` 中的封闭常量 `PLUGIN_SDK_JSONRPC`。该插件按 `agents` 注入 `AgentRegistry`、按 `sessions` 注入 `JsonlSessionStore`（不是 `Arc<_>`），绑定 stdin/stdout，提供 `sdkJsonRpcServer`，并在 `shutdown` 之后安装进程退出钩子。bin 在 `boot_yaml` 之后才开始 serve，因此 `initialize` 能看到兄弟插件的 `register_adapter`。测试在进程内构造 `HarnessSdkJsonRpcServer`，不设置退出钩子。

方法为 `initialize`、`session/prompt` 与 `shutdown`。`initialize` 应答 `serverInfo.name = deepseek-harness-sdk-runtime` 以及 `version = 0.0.1`。若存在 `maxTokens`，它必须是正整数。再次 initialize 返回 `Err("re-initialize is unsupported")`。缺失的提供方会大声失败，包括 `deepseek-official`；第 5 阶段不会从 `initialize` 挂载实时 DeepSeek 适配器。`session/prompt` 在 `followup` 之后、`when_idle` 完成之前返回 `{messageId}`；第一个未知会话 id 调用 `AgentRegistry::create`。通知为 `session.event`（完整 `SessionEvent` 信封）与 `session.status`（`idle` 或 `running`）。单一 FIFO 任务按入队顺序写出它们，因此 idle 不会抢在 inbox splice 或 assistant 文本之前（否则 TypeScript SDK 在等待 splice 时会跳过过早的 idle 然后挂起，或返回空的 `finalResponse`）。服务器从不发出 `subagent.started` 或 `subagent.finished`。stdout 只承载帧。

该 bin 依次调用 `dsh_agent::register_spine_plugins`、`register_execution_plugins`、`dsh_base::register_base_plugins` 与本 crate 的 `register`，然后使用一份与 `dsh-cli` 相同规则的本地 `ensure_persist_env`（非空的 `DSH_SESSION_ROOT` 或 `DSH_HOME` 为无操作；否则 `DSH_HOME=$HOME/.dsh`；缺少 `HOME` 时向 stderr 打印并以退出码 1 退出）。当 `$DSH_CORDIS_CONFIG` 已设置且非空时使用该路径的配置 YAML，否则使用附带的 `minimal.cordis.yml`。该文件不得包含子串 `!!js`。其 mock 适配器占用 `deepseek-official`，因此无密钥 initialize 可用。插件在 setup 中绑定并提供 `sdkJsonRpcServer`，不在 setup 内 serve；bin 在 `boot_yaml` 返回之后调用 `serve`。

## 备选方案

**给 `dsh-boot` 增加产品 crate 依赖，以便在那里注册 JSON-RPC 插件。** 否决：这会再次形成 [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) 要防止的环。

**从 persist、boot 或 `dsh-cli` 导出 `ensure_persist_env`。** 本阶段否决：bin 保留本地副本，避免那些 crate 增加面向 JSON-RPC 的 API。

**从 `initialize` 挂载实时 DeepSeek 适配器。** 第 5 阶段否决：YAML 已经把 `deepseek-official` 注册为 mock 或 replay；提供方缺失仍然大声失败。

**因为协议 crate 定义了 `subagent.started` / `subagent.finished`，就从本服务器发出它们。** 否决：第 5 阶段没有同进程 subagent；具名载荷存在是为了让客户端能编译，本服务器从不构造它们。

## 影响

`cargo test -p dsh-sdk-jsonrpc-server --offline` 覆盖稳定的 `serverInfo.name`、在 Hang 结束前就返回 `messageId` 的惰性 `session/prompt`、文本 mock 之后的 `session.status` idle，以及不支持的再次 initialize。`cargo build -p dsh-sdk-jsonrpc-server --offline` 生成 `target/debug/dsh-jsonrpc-agent`。Vitest 在 `DSH_RUNTIME=rust` 下为 `text-turn` 与 `bash-tool` 拉起该 bin。

## 相关

协议类型与传输见 [dsh-sdk-protocol](../../../../crates/dsh-sdk-protocol/README.md)。spine 与执行插件注册见 [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) 与 [Execution YAML plugins](2026-08-16-execution-plugins-in-dsh-agent.md)。第 6 阶段产品名称见 [dsh-base](2026-08-16-rust-dsh-base-plugins.md)。快照驱动拉起该 bin 见 [Rust snapshot harness](../../proposed/testing/2026-08-15-rust-snapshot-harness.md)。
