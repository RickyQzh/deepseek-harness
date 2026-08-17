# dsh-mcp-client

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 MCP 客户端。本 crate 是作为 MCP 客户端的 harness，不是 MCP 服务器，也不是 ACP（Agent Client Protocol）适配器（`dsh-acp`）。MCP stdio 使用 LSP Content-Length JSON-RPC 2.0，不是 ACP 或 SDK NDJSON。

`register` 挂载 YAML `@deepseek-ai/dsh-mcp-client`（`dsh_boot::PLUGIN_MCP_CLIENT`）。`register_mcp_plugins` 调用 `register`。插件 setup 返回 `Ok(())`，不建立连接。

`public_tool_name(server_name, raw_name)` 是 TypeScript `publicToolName(serverName, rawName)`：当 `mcp__<serverName>__<rawName>` 已满足 `[A-Za-z0-9_-]` 且长度不超过 64 时原样返回；否则把字符集规范化后的形式截到 51 个字符，再追加 `_` 与 `serverName + NUL + rawName` 的 12 位十六进制 SHA-256。`extract_text` 用换行拼接 MCP `text` 块，并把 image、audio、resource 及其他块换成占位符；空内容为 `(<tool_name> returned no text content)`。

本 crate 不依赖 `dsh-acp`、`dsh-sdk-protocol`、`dsh-agent`、`dsh-host`、`dsh-cli`、`dsh-headless`、`rmcp` 或 `native-tls`。`dsh-agent` 不得依赖本 crate。命名冻结见 [Rust MCP 客户端 Agent Note](../../.agents/notes/proposed/architecture/2026-08-17-rust-mcp-client.md)。

## 已知限制与暂缓事项

- 不支持 Streamable HTTP。
- 不支持 MCP Resources 与 Prompts。
- 省略具名快照；覆盖为 crate 测试。
- 不采用官方 `rmcp`（workspace rustc 为 1.85）。
