# dsh-acp

English | [中文](README.zh.md)

ACP stdio adapter for the DeepSeek Harness Rust host. This crate is not the SDK JSON-RPC server (`dsh-sdk-jsonrpc-server`). Framing is NDJSON JSON-RPC 2.0, not LSP Content-Length. Stdout carries those ACP frames only.

`register` mounts YAML `@deepseek-ai/dsh-acp` (`dsh_boot::PLUGIN_ACP`). `register_acp_plugins` calls `register`. Plugin setup returns `Ok(())` after rejecting unknown config keys and does not serve stdio. The kernel service name is `acpServer`. This crate does not depend on `dsh-sdk-protocol`, `dsh-sdk-jsonrpc-server`, `dsh-base`, `dsh-host`, `dsh-cli`, `dsh-headless`, or `dsh-subagent`. `dsh-agent` must not depend on this crate.

`initialize` ignores the client's `protocolVersion` and always returns `protocolVersion` 1, `agentInfo.name` `deepseek-harness-acp`, `agentInfo.version` `0.0.1`, empty `authMethods`, and `promptCapabilities` with `image`, `audio`, and `embeddedContext` all false; `sessionCapabilities` and `mcpCapabilities` are omitted. `authenticate` returns `{}`. `session/new` requires an absolute `cwd`; empty `additionalDirectories` and `mcpServers` are accepted; non-empty lists return JSON-RPC `-32602`. Unknown requests, including `session/load`, return JSON-RPC `-32601` with message `"Method not found": {method}` and `data.method`.

`acp_prompt_to_text` concatenates `text` blocks verbatim and renders each `resource_link` as a bracketed `[resource_link name=<json-string> uri=<json-string>]` reference; other blocks contribute nothing. `prompt_has_unsupported_content` is true when any block is neither `text` nor `resource_link`. `turn_end_to_stop_reason` maps harness `TurnEndReason` to ACP `StopReason` (`MaxTokens` → `max_tokens`). `prompt_stop_reason` is the prompt-RPC map: missing end reason → `cancelled`, `MaxTokens` → `end_turn`, otherwise `turn_end_to_stop_reason`.

`session/prompt` waits for whole-agent idle, emits committed `agent_message_chunk` text, and reports `end_turn` (token-limit included).

## Known Limitations and Deferred Work

- Session load, list, resume, delete, and fork are unsupported.
- MCP, PTY, and LSP are unsupported.
- Continuable descendant drain is unsupported.
- Named snapshot subset is handshake, reject-extra-dirs, and text-turn only.
