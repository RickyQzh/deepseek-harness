# Agent Note: Freeze the Rust LSP stdio host and named ACP lsp-definition scenario

Status: proposed

English | [中文](2026-08-17-rust-lsp-host.zh.md)

## Problem

The [Rust rewrite](2026-08-14-rust-rewrite.md) Phase 8 fourth item is an LSP stdio host (transient open, no `workspace/applyEdit`). TypeScript already ships three packages (`@deepseek-ai/dsh-lsp`, `@deepseek-ai/dsh-lsp-stdio`, `@deepseek-ai/dsh-tool-lsp`), a closed four-operation `lsp` tool, and a named ACP snapshot `lsp-definition`. The Rust host has no LSP crates. Without a freeze, a port can collapse the three roles, depend on `tower-lsp` / `async-lsp` / `lsp-types` / `rmcp`, reuse `dsh-mcp-client` Content-Length framing, send the host PID instead of `processId: null`, apply `workspace/applyEdit`, feed `didOpen` from CRLF-normalized `read_text`, mount LSP on shared `rust.snapshot.cordis.yml`, add `--profile lsp`, or put LSP rows in default YAML.

## Proposal

Phase 8 item 4 implements the existing LSP capability on the Rust host as YAML rows named `@deepseek-ai/dsh-lsp`, `@deepseek-ai/dsh-lsp-stdio`, and `@deepseek-ai/dsh-tool-lsp`. Keep three crates. The model-facing tool name is `lsp`. Operations are the closed union `goToDefinition` | `findReferences` | `goToImplementation` | `hover`. Cite the [rewrite note](2026-08-14-rust-rewrite.md) for keep-or-drop of Cordis, Landlock, `!!js`, session format, rusqlite, and `native/landlock-run`. Phase 8 item 4 does not add rusqlite, does not port `!!js`, does not rewrite `landlock-run`, does not depend on `tower-lsp`, `async-lsp`, `lsp-types`, or `rmcp`, and does not edit [docs/architecture.md](../../../../docs/architecture.md).

This note does not supersede [LSP capability seam](../../implemented/architecture/2026-07-15-lsp-capability-seam.md). That note remains the TypeScript contract owner. This note records the Rust stdio subset and the named Vitest cutover.

`register_base_plugins` registers the three plugin types so `--patch` / overlay can mount them. Default headless, ACP, web, and jsonrpc compositions omit LSP rows and must not spawn a language-server child. There is no `--profile lsp`.

## Wire freeze

Duplicate Content-Length JSON-RPC framing inside `dsh-lsp-stdio` (`Content-Length: {n}\r\n\r\n`). Do not import `dsh-mcp-client` framing. Do not extract `dsh-content-length`. ACP stays NDJSON. MCP stays Content-Length in `dsh-mcp-client`. Initialize sends `processId: null` (JSON null, not omitted, not the host PID). `workspace/applyEdit` is refused with JSON-RPC error code `-32601` and message `workspace/applyEdit is not permitted by this host`; the query keeps serving. The host never applies the edit and never runs `workspace/executeCommand`. `LspService` exposes only `register_provider` and `query`. `didOpen` text comes from `LocalFileSystem::stream_text` (raw UTF-8 chunks, no CRLF rewrite). LSP must not call `read_text`. The snapshot read must not emit `fs/observed`. Spawn uses `SubprocessHandle` Pipe stdin/stdout (not a second tokio `Command`). `dsh-tool-lsp` reads `session_cwd` on `ToolExecution` only and must not depend on `dsh-agent`. `dsh-agent` must not depend on any LSP crate.

Named ACP `lsp-definition` uses overlay `examples/acp-agent/rust.lsp.snapshot.cordis.yml` (`!!js`-free, fixture bin argv, `maxLocations: 1`). Shared `rust.snapshot.cordis.yml` stays without LSP rows.

## Phase 8 LSP subset

| Surface | Rust Phase 8 item 4 | Stays TypeScript / later |
|---|---|---|
| ACP `lsp-definition` | Vitest when `DSH_RUNTIME=rust` | |
| Protocol / framing / applyEdit / jail / UTF-16 / `includeDeclaration` | crate tests + in-crate Content-Length fixture | |
| Real `typescript-language-server` e2e | out | TypeScript `typescript-server.e2e.ts` |
| Shared `rust.snapshot.cordis.yml` | stay without LSP rows | |
| Full `pnpm run test:snapshot` on Rust | out | rewrite-program exit |

## Alternatives considered

**Depend on `tower-lsp`, `async-lsp`, or `lsp-types`.** Rejected: `tower-lsp` is a server framework; `async-lsp` exposes `apply_edit`; `lsp-types` is unmaintained and covers the whole spec. Workspace rustc is 1.85. The host is an owned thin client.

**Depend on `rmcp` for JSON-RPC.** Rejected: `rmcp` is MCP, not LSP, and already fails the 1.85 MSRV pin.

**Import `dsh-mcp-client::rpc` or extract `dsh-content-length`.** Rejected: LSP needs header and message caps MCP lacks; a shared crate would retouch shipped MCP; MCP error strings must not leak into LSP diagnostics.

**Use `read_text` for `didOpen`.** Rejected: `read_text` rewrites `\r\n` to `\n` and would shift UTF-16 columns on CRLF files.

**Mount LSP on shared `examples/acp-agent/rust.snapshot.cordis.yml`.** Rejected: handshake and `text-turn` schema pins would gain the `lsp` tool. Overlay `rust.lsp.snapshot.cordis.yml` is the cutover file.

**Add `--profile lsp` or default YAML rows.** Rejected: implemented profiles stay `headless` / `web` / `acp`. A shipped default would spawn a language-server child on every headless run.

**Collapse the three roles into one crate.** Rejected: the rewrite keeps Definition / Provider / Consumer separate when those roles evolve independently. TypeScript already split them.

**Accept `workspace/applyEdit` because some servers send it.** Rejected: the TypeScript host refuses the request with `-32601` and keeps serving. The host must never apply the edit.

## Acceptance criteria

- The rewrite note follow-up table links to this file.
- YAML names are `@deepseek-ai/dsh-lsp`, `@deepseek-ai/dsh-lsp-stdio`, and `@deepseek-ai/dsh-tool-lsp`; operations stay the four-arm closed union.
- `processId` is JSON `null`; `workspace/applyEdit` is `-32601`; Content-Length lives in `dsh-lsp-stdio` only.
- `didOpen` uses `stream_text`; spawn uses Pipe handles; `session_cwd` is on `ToolExecution`.
- Named Vitest ACP subset gains `lsp-definition`; overlay is `rust.lsp.snapshot.cordis.yml`; shared rust ACP YAML stays without LSP rows; default YAML omits LSP; there is no `--profile lsp`.
- [docs/architecture.md](../../../../docs/architecture.md) is not edited.
- This note does not supersede the TypeScript LSP capability-seam note.

## Risks

A reviewer may treat ACP NDJSON or MCP framing as the language-server codec. The frames differ; LSP duplicates Content-Length in `dsh-lsp-stdio`.

A reviewer may add LSP rows to shared `rust.snapshot.cordis.yml`. Handshake schemas would drift.

A reviewer may accept `workspace/applyEdit` to behave as a full editor client. The host must refuse `-32601` and never apply the edit.
