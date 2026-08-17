# dsh-mcp-client

English | [中文](README.zh.md)

MCP client for the DeepSeek Harness Rust host. This crate is the harness as an MCP client, not an MCP server and not the ACP adapter (`dsh-acp`). MCP stdio uses LSP Content-Length JSON-RPC 2.0, not ACP or SDK NDJSON.

`register` mounts YAML `@deepseek-ai/dsh-mcp-client` (`dsh_boot::PLUGIN_MCP_CLIENT`). `register_mcp_plugins` calls `register`. Plugin setup parses stdio config (unknown keys fail load; `transport: streamable-http` fails with `mcp-client: streamable-http is not supported in Phase 8 item 2`), injects `tools`, reserves `serverName` on that runtime, spawns the child, sends `initialize`, and registers tools under `mcp__` public names. A duplicate live `serverName` fails load. `failOnStartupError` defaults to false: spawn, initialize, and sync failures log and return `Ok(())`. Config, reservation, and HTTP errors always fail load. Optional `reconnect` must be an object; its fields are not parsed yet.

`sync_tools` lists tools, refuses a repeated raw name without touching `previous`, refuses a foreign public-name squat without touching `previous`, otherwise unregisters `previous` and `try_register`s each `mcp__` name. Executors send the raw name on `tools/call` (non-object args become `{}`); `isError: true` is `ToolError::Other` from `extract_text` (public name in the empty-content fallback); `taskSupport: required` is `ToolError::Other` with `Tool "RAW" requires task-based execution, which this bridge does not support`. Native `render` is `extract_text` as one text content block.

`McpSession` sends `initialize` with `protocolVersion` `2025-03-26`, `capabilities` `{}`, and `clientInfo` `dsh-mcp-client`/`0.0.1`, then notification `notifications/initialized` (method set, no `id`); `list_tools` repeats `tools/list` until `nextCursor` is absent; `call_tool` sends the raw MCP name on `tools/call`. `McpToolDraft` accessors return listed name, description, input schema, and `task_required` (true only when the server advertises execution taskSupport `required`). `McpSession::from_stdio` binds already-open server stdout and stdin byte streams. `stdio_child_env` is `dsh_subprocess::child_env` over the `config.env` overlay, so parent credential-shaped names such as `DEEPSEEK_API_KEY` are absent unless the overlay restores them. `stdio_command` builds `Command::new(program).args(args)` (no shell) with `env_clear().envs(env)`; empty or omitted `cwd` inherits. `spawn_stdio` spawns that command, takes child stdin (write) and stdout (read), and wraps them with `from_stdio`.

`public_tool_name(server_name, raw_name)` is the TypeScript `publicToolName` function of `(serverName, rawName)`: `mcp__<serverName>__<rawName>` when that string already matches `[A-Za-z0-9_-]` and is at most 64 characters; otherwise the charset-normalized form is truncated to 51 characters and `_` plus a 12-hex SHA-256 of `serverName + NUL + rawName` is appended. `extract_text` joins MCP `text` blocks with newlines and replaces image, audio, resource, and other blocks with placeholders; empty content is `(<tool_name> returned no text content)`.

This crate does not depend on `dsh-acp`, `dsh-sdk-protocol`, `dsh-agent`, `dsh-host`, `dsh-cli`, `dsh-headless`, `rmcp`, or `native-tls`. `dsh-agent` must not depend on this crate. Naming freeze: [Rust MCP client Agent Note](../../.agents/notes/proposed/architecture/2026-08-17-rust-mcp-client.md).

## Known Limitations and Deferred Work

- Streamable HTTP is unsupported.
- MCP Resources and Prompts are unsupported.
- Named snapshots are omitted; coverage is crate tests.
- Official `rmcp` is out (workspace rustc 1.85).
