# dsh-acp

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 ACP（Agent Client Protocol）stdio 适配器。本 crate 不是 SDK JSON-RPC 服务器（`dsh-sdk-jsonrpc-server`）。帧格式为 NDJSON JSON-RPC 2.0，不是 LSP Content-Length。stdout 只承载这些 ACP 帧。

`register` 挂载 YAML `@deepseek-ai/dsh-acp`（`dsh_boot::PLUGIN_ACP`）。`register_acp_plugins` 调用 `register`。插件 setup 在拒绝未知配置键后返回 `Ok(())`，并且不 serve stdio。内核服务名为 `acpServer`。本 crate 不依赖 `dsh-sdk-protocol`、`dsh-sdk-jsonrpc-server`、`dsh-base`、`dsh-host`、`dsh-cli`、`dsh-headless` 或 `dsh-subagent`。`dsh-agent` 不得依赖本 crate。

`initialize` 忽略客户端的 `protocolVersion`，始终返回 `protocolVersion` 1、`agentInfo.name` 为 `deepseek-harness-acp`、`agentInfo.version` 为 `0.0.1`、空的 `authMethods`，以及 `image`、`audio` 与 `embeddedContext` 均为 false 的 `promptCapabilities`；不序列化 `sessionCapabilities` 与 `mcpCapabilities`。`authenticate` 返回 `{}`。未知请求（包括 `session/load`）返回 JSON-RPC `-32601`，消息为 `"Method not found": {method}`，并带有 `data.method`。

## 已知限制与暂缓事项

- 不支持会话 load、list、resume、delete 与 fork。
- 不支持 MCP、PTY 与 LSP。
- 不支持可继续后代 drain。
- 具名快照子集仅为 handshake、reject-extra-dirs 与 text-turn。
