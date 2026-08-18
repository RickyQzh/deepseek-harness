# dsh-rpc

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 GUI 四象限 RPC 信封。本 crate 是 GUI 协议，不是 SDK JSON-RPC（`dsh-sdk-protocol`）。

`RpcMessage` 是带内部标签的联合类型 `client-request` | `server-response` | `server-request` | `client-response`。字段名为 camelCase（`rpcId`）。应答的 `result` 是 `RpcResult`：`{ "ok": true, "value": ... }` 或 `{ "ok": false, "error": { "code", "message", "details" } }`。`details` 必填；`{}` 合法。错误 `code` 是封闭的 kebab-case 集合；未知 code 解码失败。`RpcId` 是本地 newtype（`new` / `as_str`）。

`RpcReceipt` 是载体回执，不是 `RpcMessage`：`{ "accepted": true }` 或 `{ "accepted": false, "reason": "not-pending" | "bad-response" }`。它没有 `type` 字段。

其他 crate 通过访问器读取解码后的信封（`rpc_id`、`method`、`payload`、`result`、`as_ok`/`as_err`、`is_accepted`/`reject_reason`）；变体字段为私有，因此在本 crate 外按字段匹配无法编译。

载荷与错误 details 保持为 `serde_json::Value`。本 crate 没有 HTTP 服务器，也不依赖 `dsh-session`。

## 已知限制与暂缓事项

- 一元 HTTP 分发、WebSocket 帧与方法处理函数在 `dsh-host`，不在本 crate。
- 不移植 TypeScript 按 code 划分的 details 结构体；details 仍是 JSON 值。
