# Agent Note: Freeze the Rust ACP stdio subset and name the Phase 8 Vitest scenarios

Status: proposed

English | [中文](2026-08-17-rust-acp-server.zh.md)

## Problem

The [Rust rewrite](2026-08-14-rust-rewrite.md) Phase 8 first item is an ACP server (byte-compatible subset). TypeScript `@deepseek-ai/dsh-acp` already implements an automation-only Agent Client Protocol bridge over JSON-RPC stdio. Phase 5/6 shipped a different NDJSON JSON-RPC runtime (`dsh-sdk-jsonrpc-server`, `serverInfo.name = deepseek-harness-sdk-runtime`). Without a freeze, a Rust port can mix those protocols, use LSP Content-Length framing, treat `dsh --profile acp` as still unimplemented, or claim the full `examples/acp-agent` snapshot matrix as the cutover.

## Proposal

Phase 8 item 1 implements the existing automation-only ACP contract on the Rust `dsh` binary as `--profile acp` / argv1 `acp`. Framing is newline-delimited JSON-RPC 2.0 (`ndJsonStream`), not LSP `Content-Length`. `agentInfo.name` is `deepseek-harness-acp` and `agentInfo.version` is `0.0.1`. `protocolVersion` is `1`. Cite the [rewrite note](2026-08-14-rust-rewrite.md) for keep-or-drop of Cordis, Landlock, `!!js`, session format, rusqlite, and `native/landlock-run` rather than restating those rows here. Phase 8 item 1 does not add rusqlite, does not port `!!js`, does not rewrite `landlock-run`, and does not edit [docs/architecture.md](../../../../docs/architecture.md).

This note does not supersede [ACP as an automation-only protocol](../../implemented/simplification/2026-07-23-acp-automation-only-protocol.md). That note remains the TypeScript contract owner. This note records the Rust subset and the named Vitest cutover.

Implemented client→agent methods are `initialize`, `authenticate` (no-op `{}`), `session/new`, and `session/prompt`. The implemented notification is `session/cancel`. Outbound methods are `session/update` (`agent_message_chunk` text only) and `session/request_permission` (one-shot `allow-once` / `reject-once`). Every other ACP request returns JSON-RPC `-32601` with message `"Method not found": <method>`. Validation failures return `-32602` with the TypeScript `Invalid params: …` detail strings. A correlated turn error rejects `session/prompt` with `-32603`.

Phase 8 item 1 keeps `pnpm run demo:acp` as the TypeScript example. Named scenarios spawn `target/debug/dsh --profile acp` when `DSH_RUNTIME=rust`, and are recorded in the [snapshot-harness note](../testing/2026-08-15-rust-snapshot-harness.md). Remaining ACP snapshot scenarios stay on the Node `dsh-acp-demo` bin and reuse the [ACP snapshot tests](../../implemented/testing/2026-06-19-acp-snapshot-tests.md) fixture directories. Full `pnpm run test:snapshot` against Rust is not Phase 8 item 1's exit.

## Wire freeze

Transport is one UTF-8 JSON object per line. Stdout carries only those frames. Diagnostics go to stderr. There is no ready URL line.

`session/prompt` blocks until whole-agent idle and returns `{ "stopReason": … }`. Token-limit turn endings settle as `end_turn`. Explicit cancel, disposal, or a turnless slot settles as `cancelled`. The codec maps `max-tokens` to `max_tokens` for other callers; the prompt RPC does not use that value.

Permission answers never become durable grants. Missing `callId` or a session the bridge does not own delegates with `next()`. A client error fails closed to `unavailable`. Unknown `optionId` is `rejected`.

## Phase 8 ACP subset

| Scenario | Driver | Bin | Fixture dir |
|---|---|---|---|
| ACP `handshake` | Vitest `examples/acp-agent/tests/acp.snapshot.ts` | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/acp-agent/tests/snapshots/handshake/` |
| ACP `reject-extra-dirs` | same | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/acp-agent/tests/snapshots/reject-extra-dirs/` |
| ACP `text-turn` | same | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/acp-agent/tests/snapshots/text-turn/` |
| remaining ACP scenarios | same suite | Node only | existing dirs |

## Alternatives considered

**Use LSP Content-Length because the public ACP spec examples often show it.** Rejected: this repository's TypeScript server, snapshot launcher, subagent client, and handshake goldens are `ndJsonStream` NDJSON.

**Route ACP through `dsh-sdk-jsonrpc-server` because both are NDJSON JSON-RPC.** Rejected: `session/prompt` semantics, session lifecycle, notifications, and process identity differ. Sharing a dispatcher would mix wires.

**Ship a `dsh-acp-agent` bin parallel to `dsh-jsonrpc-agent`.** Rejected for Phase 8 item 1: ACP clients spawn a configured command; the rewrite puts ACP on the one `dsh` host. Python does not Popen an ACP-specific name.

**Treat full `examples/acp-agent` snapshots as the cutover.** Rejected: that corpus pins backend tools, PTY, LSP, workflow, Code Mode, hooks, and subagents. Phase 8 item 1 names three protocol scenarios. The same named-subset pattern as Phase 5/6/7.

**Skip `authenticate` because `authMethods` is empty.** Rejected: TypeScript implements a no-op and tests call it.

**Depend on `dsh-subagent` for continuable drain.** Rejected for Phase 8 item 1: TypeScript avoids that package dependency by a structural `ctx.get`. Out-of-process subagents are a later Phase 8 item.

## Acceptance criteria

- The rewrite note follow-up table links to this file.
- Framing is NDJSON JSON-RPC 2.0; `agentInfo.name` is `deepseek-harness-acp`.
- Implemented methods are the five listed above plus two outbound methods; other requests are `-32601` with the TypeScript message.
- Named Vitest ACP subset is `handshake`, `reject-extra-dirs`, and `text-turn`; remaining ACP scenarios stay Node. This note does not claim the full ACP snapshot matrix on Rust.
- [docs/architecture.md](../../../../docs/architecture.md) is not edited.
- This note does not supersede the automation-only ACP note.

## Risks

A reviewer may treat the named subset as full `pnpm run test:snapshot` on Rust. Remaining scenarios stay Node.

A reviewer may point Python or the GUI at ACP stdio. Those wires stay SDK NDJSON and four-quadrant HTTP/WS.

`dsh --profile acp` without `headless-auto-approve` fails closed on tool approval unless the client answers `session/request_permission`. That is the TypeScript contract.
