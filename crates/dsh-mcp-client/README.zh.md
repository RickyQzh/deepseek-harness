# dsh-mcp-client

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 MCP 客户端。本 crate 是作为 MCP 客户端的 harness，不是 MCP 服务器，也不是 ACP（Agent Client Protocol）适配器（`dsh-acp`）。MCP stdio 使用 LSP Content-Length JSON-RPC 2.0，不是 ACP 或 SDK NDJSON。

`register` 挂载 YAML `@deepseek-ai/dsh-mcp-client`（`dsh_boot::PLUGIN_MCP_CLIENT`）。`register_mcp_plugins` 调用 `register`。插件 setup 解析 stdio 配置（未知键导致加载失败；`transport: streamable-http` 以 `mcp-client: streamable-http is not supported in Phase 8 item 2` 失败），注入 `tools`，在该运行时上预留 `serverName`，启动重连监督器，并在激活前等待第一代 `ready`。第二个仍存活的相同 `serverName` 导致加载失败。`failOnStartupError` 默认为 false：首次尝试失败会记录日志并让监督器继续循环；为 true 时返回 `SetupFailed`。配置、预留、HTTP 与重连策略错误始终导致加载失败。`resolve_reconnect_policy` 使用前缀 `mcp-client({serverName}): reconnect`，默认 `enabled` 为 true、`initialDelayMs` 为 500、`maxDelayMs` 为 30000、`maxAttempts` 为 10。成功 initialize 之后，子进程退出或读到 EOF 时，若启用重连则按 `min(maxDelayMs, initialDelayMs * 2^(failedAttempts-1))` 退避；自 `connectedAt` 起的存活时间不少于 `maxDelayMs` 时，在递增前将 `failedAttempts` 重置为 0。中断期间上一世代保持注册，调用失败。连续失败达到 `maxAttempts` 后，监督器注销已拥有的公开名称并停止。`reconnect.enabled: false` 不会重新 spawn。Dispose 取消退避、杀死子进程、最多等待 5000 ms、注销剩余名称，并释放 `serverName`。

`sync_tools` 列出工具；若原始名称重复则拒绝且不改动 `previous`；若公开名称被外部占用则拒绝且不改动 `previous`；否则注销 `previous` 并对每个 `mcp__` 名称调用 `try_register`。执行器在 `tools/call` 上发送原始名称（非对象参数变为 `{}`）；`isError: true` 变为来自 `extract_text` 的 `ToolError::Other`（空内容回退使用公开名称）；`taskSupport: required` 变为 `ToolError::Other`，文案为 `Tool "RAW" requires task-based execution, which this bridge does not support`。Native `render` 把 `extract_text` 做成一个文本内容块。

`McpSession` 发送 `initialize`（`protocolVersion` 为 `2025-03-26`，`capabilities` 为 `{}`，`clientInfo` 为 `dsh-mcp-client`/`0.0.1`），随后发送通知 `notifications/initialized`（有 method、无 `id`）；`list_tools` 重复 `tools/list` 直到 `nextCursor` 缺席；`call_tool` 在 `tools/call` 上发送原始 MCP 名称。`McpToolDraft` 访问器返回列出的名称、描述、输入 schema，以及 `task_required`（仅当服务器声明 execution taskSupport 为 `required` 时为 true）。`McpSession::from_stdio` 绑定已打开的服务器 stdout 与 stdin 字节流。`stdio_child_env` 是对 `config.env` overlay 调用 `dsh_subprocess::child_env`，因此父进程中形似凭据的名称（如 `DEEPSEEK_API_KEY`）不会出现，除非 overlay 将其恢复。`stdio_command` 构建 `Command::new(program).args(args)`（不经过 shell），并使用 `env_clear().envs(env)`；`cwd` 为空或省略时继承。`spawn_stdio` 按该命令 spawn，取出子进程 stdin（写入）与 stdout（读取），并用 `from_stdio` 包装。

`public_tool_name(server_name, raw_name)` 是 TypeScript `publicToolName(serverName, rawName)`：当 `mcp__<serverName>__<rawName>` 已满足 `[A-Za-z0-9_-]` 且长度不超过 64 时原样返回；否则把字符集规范化后的形式截到 51 个字符，再追加 `_` 与 `serverName + NUL + rawName` 的 12 位十六进制 SHA-256。`extract_text` 用换行拼接 MCP `text` 块，并把 image、audio、resource 及其他块换成占位符；空内容为 `(<tool_name> returned no text content)`。

本 crate 不依赖 `dsh-acp`、`dsh-sdk-protocol`、`dsh-agent`、`dsh-host`、`dsh-cli`、`dsh-headless`、`rmcp` 或 `native-tls`。`dsh-agent` 不得依赖本 crate。命名冻结见 [Rust MCP 客户端 Agent Note](../../.agents/notes/proposed/architecture/2026-08-17-rust-mcp-client.md)。

## 已知限制与暂缓事项

- 不支持 Streamable HTTP。
- 不支持 MCP Resources 与 Prompts。
- 省略具名快照；覆盖为 crate 测试。
- 不采用官方 `rmcp`（workspace rustc 为 1.85）。
