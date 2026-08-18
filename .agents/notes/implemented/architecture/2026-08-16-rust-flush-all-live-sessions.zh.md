# Agent Note: 在 when_idle 之后 flush AgentRegistry 中每一个实时会话

Status: implemented

[English](2026-08-16-rust-flush-all-live-sessions.md) | 中文

## 问题

Headless 与 JSON-RPC 在 `when_idle` 之后做持久化，以便一次性进程留下 `{DSH_SESSION_ROOT}/{id}/session.jsonl`。可继续的 child 仍注册在 `AgentRegistry` 上，并各自持有 `Session` 日志。若 `when_idle` 之后只对当前任务会话调用 `flush`，这些 child 就不会得到目录，settlement 与 resume 也就无法观察到 child 的 `{id}/session.jsonl`。

## 决策

`AgentRegistry::list` 以未指定顺序返回实时 `AgentHandle` 的快照。`when_idle` 返回后，`dsh-headless` 的 `headless-runner` 与 `dsh-sdk-jsonrpc-server` 的 `session/prompt` 遍历 `list()`，并对每个实时 `Session` 调用 `JsonlSessionStore::flush`。每一个仍注册的会话（包括可继续 child）都会写入 `{DSH_SESSION_ROOT}/{id}/session.jsonl`。父会话 idle 时不会 dispose（资源释放） child。

## 测试

`DSH_RUNTIME=rust` 的 headless `subagent-settlement` 断言 `{cwd}/.sessions` 下至少有两个目录。`AgentRegistry::list` 没有 crate 本地测试。

## 考虑过的替代方案

**只 flush 当前任务会话。** 不予采用：可继续 child 在 `when_idle` 之后永远得不到 `{id}/session.jsonl`。

**在每次会话追加时 flush。** 不予采用：idle 已是现有的耐久性屏障；按事件 flush 会增加 I/O，而进程退出时并没有第二个消费者。

**在父会话 idle 时 dispose child，再 flush 剩余者。** 不予采用：可继续 child 必须保持注册，供 `send_message` 与 settlement 使用。

**让每个 child 在自己进入 idle 时 flush。** 不予采用：父宿主已经拥有进程退出时的持久化；第二个写入者会与同一 JSONL 文件竞态。

## 后果

在父会话的 `when_idle` 返回之前就从注册表移除的 child 不会在这里被 flush。JSON-RPC 服务器忽略单次 flush 错误（`let _ =`）；headless 返回第一次 flush 错误。调用方不得依赖 `list` 的顺序。

## 相关

JSON-RPC 宿主见 [Rust SDK JSON-RPC server](2026-08-16-rust-sdk-jsonrpc-server.md)。`list` 记载于 [dsh-agent](../../../../crates/dsh-agent/README.md)。快照持久化路径与 rust settlement 覆盖见 [Rust snapshot harness](../../proposed/testing/2026-08-15-rust-snapshot-harness.md)。
