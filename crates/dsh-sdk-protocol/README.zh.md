# dsh-sdk-protocol

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 SDK JSON-RPC 2.0 请求与通知载荷。

JSON-RPC 方法名为 `initialize`、`session/prompt` 和 `shutdown`。握手 `serverInfo.name` 是协议稳定字符串 `deepseek-harness-sdk-runtime`。`ContentBlock` 与 `SessionEvent` 字段复用 `dsh-session` 类型及其 camelCase serde。

通知载荷 `session.event`、`session.status`、`subagent.started` 与 `subagent.finished` 存在，以使协议类型完整。第 5 阶段服务器可能从不发出 `subagent.started` 或 `subagent.finished`。

## 已知限制与暂缓事项

- NDJSON 分帧、stdio 传输与 JSON-RPC 方法分发不在本 crate；它们在 Task 47 落地。
