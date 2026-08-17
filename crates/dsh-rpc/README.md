# dsh-rpc

English | [中文](README.zh.md)

GUI four-quadrant RPC envelopes for the DeepSeek Harness Rust host. This crate is the GUI wire, not SDK JSON-RPC (`dsh-sdk-protocol`).

`RpcMessage` is the internally tagged union `client-request` | `server-response` | `server-request` | `client-response`. Field names are camelCase (`rpcId`). A response `result` is `RpcResult`: `{ "ok": true, "value": ... }` or `{ "ok": false, "error": { "code", "message", "details" } }`. `details` is required; `{}` is valid. Error `code` is a closed kebab-case set; unknown codes fail decode. `RpcId` is a local newtype (`new` / `as_str`).

`RpcReceipt` is a carrier receipt, not an `RpcMessage`: `{ "accepted": true }` or `{ "accepted": false, "reason": "not-pending" | "bad-response" }`. It has no `type` field.

Payload and error details stay `serde_json::Value`. This crate has no HTTP server and does not depend on `dsh-session`.

## Known Limitations and Deferred Work

- Unary HTTP dispatch, WebSocket frames, and method handlers live in `dsh-host`, not here.
- Per-code TypeScript detail structs are not ported; details remain JSON values.
