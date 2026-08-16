# dsh-sdk-protocol

English | [中文](README.zh.md)

SDK JSON-RPC 2.0 request and notification payloads for the DeepSeek Harness Rust host.

JSON-RPC method names are `initialize`, `session/prompt`, and `shutdown`. Handshake `serverInfo.name` is the wire-stable string `deepseek-harness-sdk-runtime`. `ContentBlock` and `SessionEvent` fields reuse `dsh-session` types and their camelCase serde.

Notification payloads `session.event`, `session.status`, `subagent.started`, and `subagent.finished` exist so the wire types are complete. Phase 5 servers may never emit `subagent.started` or `subagent.finished`.

## Known Limitations and Deferred Work

- NDJSON framing, stdio transport, and JSON-RPC method dispatch are not in this crate; they land in Task 47.
