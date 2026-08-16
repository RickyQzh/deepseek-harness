# dsh-sdk-protocol

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 SDK JSON-RPC 2.0 请求与通知载荷，以及 NDJSON 传输。

JSON-RPC 方法名为 `initialize`、`session/prompt` 和 `shutdown`。握手 `serverInfo.name` 是协议稳定字符串 `deepseek-harness-sdk-runtime`。`ContentBlock` 与 `SessionEvent` 字段复用 `dsh-session` 类型及其 camelCase serde。

通知载荷 `session.event`、`session.status`、`subagent.started` 与 `subagent.finished` 存在，以使协议类型完整。第 5 阶段服务器可能从不发出 `subagent.started` 或 `subagent.finished`。

`JsonRpcLineTransport` 把 JSON-RPC 2.0 分帧为每行一个以 `\n` 终止的紧凑 JSON 对象。没有 LSP `Content-Length` 分帧。格式错误的 JSON 行与非对象会被忽略。请求没有处理函数时以 `-32601` 应答，消息为 `method not found: {method}`；处理函数 `Err` 以 `-32603` 应答。未处理的通知会被丢弃。出站 `jsonrpc` 始终为 `"2.0"`。输出半边（stdout）只承载帧。

## 已知限制与暂缓事项

- JSON-RPC 方法分发作为服务器插件不在本 crate。
