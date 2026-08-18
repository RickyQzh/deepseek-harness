# Agent Note: Freeze the Rust MCP stdio client and `mcp__` public names

Status: proposed

English | [中文](2026-08-17-rust-mcp-client.zh.md)

## Problem

The [Rust rewrite](2026-08-14-rust-rewrite.md) Phase 8 second item is an MCP client (`mcp__` names). TypeScript `@deepseek-ai/dsh-mcp-client` already connects to external MCP servers, registers tools under `mcp__<serverName>__<rawName>`, and reconnects crashed stdio children. The Rust host has no such plugin. Without a freeze, a port can speak ACP NDJSON on MCP stdio, depend on `rmcp` despite rustc 1.85, mount a default MCP server in `MINIMAL_YAML`, treat Streamable HTTP or Resources as in-scope, or add snapshot scenarios the TypeScript package deliberately omitted.

## Proposal

Phase 8 item 2 implements the existing MCP **client** contract on the Rust host as YAML rows named `@deepseek-ai/dsh-mcp-client`. Each row connects to one server over stdio. Framing is LSP `Content-Length` JSON-RPC 2.0, not ACP NDJSON. `clientInfo.name` is `dsh-mcp-client` and `clientInfo.version` is `0.0.1`. Public tool names stay the TypeScript `publicToolName` function of `(serverName, rawName)`. Cite the [rewrite note](2026-08-14-rust-rewrite.md) for keep-or-drop of Cordis, Landlock, `!!js`, session format, rusqlite, and `native/landlock-run`. Phase 8 item 2 does not add rusqlite, does not port `!!js`, does not rewrite `landlock-run`, does not depend on `rmcp`, and does not edit [docs/architecture.md](../../../../docs/architecture.md).

This note does not supersede [MCP client plugin](../../implemented/feature/2026-07-07-mcp-client-plugin.md) or [MCP client auto-reconnect](../../implemented/feature/2026-08-06-mcp-client-auto-reconnect.md). Those notes remain the TypeScript contract owners. This note records the Rust stdio subset.

Implemented wire methods are `initialize`, `notifications/initialized`, paginated `tools/list`, `tools/call` with the raw MCP name, `notifications/tools/list_changed`, and `$/cancelRequest` when the tool abort signal fires. Resources, Prompts, elicitation, sampling, OAuth, and Streamable HTTP are out: a YAML `transport` other than `stdio` fails load.

`register_base_plugins` registers the plugin type so an explicit overlay can resolve `@deepseek-ai/dsh-mcp-client`. Default headless, ACP, web, and jsonrpc compositions do not mount a server row and must not spawn an MCP child. There is no `--profile mcp`.

## Wire freeze

Stdio frames are `Content-Length: <byte-length>\r\n\r\n` plus that many UTF-8 JSON bytes. Diagnostics from the harness go to stderr. The child stdin/stdout are the MCP stream. Child env starts from `scrubbed_parent_env()` and then applies `config.env`.

`serverName` is local configuration matching `^[A-Za-z0-9_-]{1,32}$`, not remote `serverInfo.name`. A second live instance with the same `serverName` on the same `tools` runtime fails load with the TypeScript duplicate-namespace sentence. `tools/call` always sends `rawName`. Public names are `mcp__<serverName>__<rawName>` except when DeepSeek function-name normalization (64 chars, `[A-Za-z0-9_-]`) changes the string, in which case a 12-hex-char SHA-256 of `serverName + NUL + rawName` is appended.

Reconnect matches the TypeScript supervisor: bounded exponential backoff, per-outage `maxAttempts`, stability window equal to `maxDelayMs`, tools stay registered during an outage, exhaustion unregisters them. `reconnect.enabled: false` keeps the v1 manual-recovery behavior. The Rust host has no HMR.

## Phase 8 MCP subset

| Surface | Rust this item | Stays TypeScript / later |
|---|---|---|
| stdio tools + `mcp__` names + reconnect | crate tests against an in-crate Content-Length fixture | |
| Streamable HTTP | out | `mcp-client.e2e.ts` HTTP |
| `@modelcontextprotocol/server-everything` / filesystem e2e | out | Node e2e |
| named Vitest snapshots | none | none (TypeScript also none) |
| `examples/mcp-memory` overlays | out (`!!js`) | Node |

## Alternatives considered

**Depend on `rmcp` 3.x because it is the official Rust MCP SDK.** Rejected for Phase 8 item 2: `rmcp` 3.1.2 declares rustc 1.88; the workspace pin is 1.85. Do not bump rustc here.

**Speak NDJSON on MCP stdio because ACP and SDK JSON-RPC already do.** Rejected: the TypeScript MCP SDK stdio transport is Content-Length. Mixing ACP framing onto MCP children is a protocol bug.

**Mount a default MCP server in `MINIMAL_YAML`.** Rejected: a shipped default would spawn a third-party child on every headless run. The plugin type is registered; the row is opt-in.

**Add named Vitest MCP snapshots.** Rejected: the TypeScript MCP note chose unit/e2e and no snapshots so system-prompt fixtures stay stable.

**Port Streamable HTTP in Phase 8 item 2.** Rejected: stdio crash recovery is the operational contract; HTTP SSE recovery stays inside the TypeScript SDK and is a later note.

## Acceptance criteria

- The rewrite note follow-up table links to this file.
- Framing is Content-Length JSON-RPC 2.0; public names match the locked `publicToolName` vectors.
- Default compositions do not spawn MCP; YAML `!!js` is absent from files Phase 8 item 2 adds.
- [docs/architecture.md](../../../../docs/architecture.md) is not edited.
- This note does not supersede the TypeScript MCP client or reconnect notes.

## Risks

A reviewer may treat ACP NDJSON as the MCP stdio codec because both are JSON-RPC. The frames differ.

A reviewer may treat `rmcp` as mandatory. The MSRV conflict is the rejection.

A reviewer may demand named snapshots. TypeScript MCP testing has none.
