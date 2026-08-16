# dsh-sdk-protocol

English | [中文](README.zh.md)

SDK JSON-RPC 2.0 request and notification payloads and NDJSON transport for the DeepSeek Harness Rust host.

JSON-RPC method names are `initialize`, `session/prompt`, and `shutdown`. Handshake `serverInfo.name` is the wire-stable string `deepseek-harness-sdk-runtime`. `ContentBlock` and `SessionEvent` fields reuse `dsh-session` types and their camelCase serde.

Notification payloads `session.event`, `session.status`, `subagent.started`, and `subagent.finished` exist so the wire types are complete. Phase 5 servers may never emit `subagent.started` or `subagent.finished`.

`JsonRpcLineTransport` frames JSON-RPC 2.0 as one compact JSON object per `\n`-terminated line. There is no LSP `Content-Length` framing. Malformed JSON lines and non-objects are ignored. A request with no handler answers `-32601` with `method not found: {method}`; a handler `Err` answers `-32603`. Unhandled notifications are dropped. Outbound `jsonrpc` is always `"2.0"`. The output half (stdout) carries frames only.

## Known Limitations and Deferred Work

- JSON-RPC method dispatch as a server plugin is not in this crate.
