# Agent Note: Rust SDK JSON-RPC server and dsh-jsonrpc-agent bin

Status: implemented

English | [中文](2026-08-16-rust-sdk-jsonrpc-server.zh.md)

## Problem

Python and TypeScript SDKs drive the harness as a stdio subprocess over NDJSON JSON-RPC 2.0. Phase 5 of the [Rust rewrite](../../proposed/architecture/2026-08-14-rust-rewrite.md) needs that same wire on a Rust binary without putting product crate dependencies in `dsh-boot`, without JavaScript YAML tags, and without mixing diagnostics onto stdout.

## Decision

`dsh-sdk-jsonrpc-server` is the SDK server plugin and the `dsh-jsonrpc-agent` bin. YAML name `sdk-jsonrpc-server` is the closed constant `PLUGIN_SDK_JSONRPC` in `dsh-boot`. The plugin injects `AgentRegistry` as `agents` and `JsonlSessionStore` as `sessions` (not `Arc<_>`), binds stdin/stdout, provides `sdkJsonRpcServer`, and installs a process-exit hook after `shutdown`. The bin serves after `boot_yaml` so `initialize` sees sibling `register_adapter`. Tests construct `HarnessSdkJsonRpcServer` in-process and leave the exit hook unset.

Methods are `initialize`, `session/prompt`, and `shutdown`. `initialize` answers `serverInfo.name = deepseek-harness-sdk-runtime` and `version = 0.0.1`. Present `maxTokens` must be a positive integer. Re-initialize returns `Err("re-initialize is unsupported")`. Missing providers fail loud, including `deepseek-official`; Phase 5 does not mount a live DeepSeek adapter from `initialize`. `session/prompt` returns `{messageId}` after `followup` and before `when_idle` completes; the first unknown session id calls `AgentRegistry::create`. Notifications are `session.event` (full `SessionEvent` envelope) and `session.status` (`idle` or `running`). A single FIFO task writes them in enqueue order so idle cannot overtake the inbox splice or assistant text (the TypeScript SDK otherwise skips an early idle while waiting for the splice, then hangs, or returns an empty `finalResponse`). The server never emits `subagent.started` or `subagent.finished`. Stdout is frames only.

The bin calls `dsh_agent::register_spine_plugins` then `register_execution_plugins` then this crate's `register`, then a local copy of `dsh-cli`'s `ensure_persist_env` (non-empty `DSH_SESSION_ROOT` or `DSH_HOME` is a no-op; else `DSH_HOME=$HOME/.dsh`; missing `HOME` prints to stderr and exits 1). Config YAML is `$DSH_CORDIS_CONFIG` when set and non-empty, otherwise bundled `minimal.cordis.yml`. That file must not contain the substring `!!js`. Its mock adapter claims `deepseek-official` so keyless initialize works. The plugin binds and provides `sdkJsonRpcServer` during setup and does not serve there; the bin calls `serve` after `boot_yaml` returns.

## Alternatives considered

**Add product crate dependencies to `dsh-boot` so the JSON-RPC plugin registers there.** Rejected: that recreates the cycle [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) exists to prevent.

**Export `ensure_persist_env` from persist, boot, or `dsh-cli`.** Rejected for this phase: the bin keeps a local duplicate so those crates do not grow a JSON-RPC-specific API.

**Mount a live DeepSeek adapter from `initialize`.** Rejected for Phase 5: YAML already registers `deepseek-official` as mock or replay; a missing provider still fails loud.

**Emit `subagent.started` / `subagent.finished` from this server because the protocol crate defines them.** Rejected: Phase 5 has no in-process subagents; typed payloads exist so clients compile, and this server never constructs them.

## Consequences

`cargo test -p dsh-sdk-jsonrpc-server --offline` covers stable `serverInfo.name`, lazy `session/prompt` returning `messageId` before Hang ends, `session.status` idle after a text mock, and unsupported re-initialize. `cargo build -p dsh-sdk-jsonrpc-server --offline` emits `target/debug/dsh-jsonrpc-agent`. Vitest `DSH_RUNTIME=rust` jsonrpc snapshots spawn that bin for `text-turn` and `bash-tool`.

## Related

Wire types and transport are [dsh-sdk-protocol](../../../../crates/dsh-sdk-protocol/README.md). Spine and execution registration are [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md) and [Execution YAML plugins](2026-08-16-execution-plugins-in-dsh-agent.md). Snapshot driver spawn of this bin is [Rust snapshot harness](../../proposed/testing/2026-08-15-rust-snapshot-harness.md).
