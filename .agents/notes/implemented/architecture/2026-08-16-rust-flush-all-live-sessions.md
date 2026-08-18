# Agent Note: Flush every live AgentRegistry session after when_idle

Status: implemented

English | [中文](2026-08-16-rust-flush-all-live-sessions.zh.md)

## Problem

Headless and JSON-RPC persist after `when_idle` so a one-shot process leaves `{DSH_SESSION_ROOT}/{id}/session.jsonl`. Continuable children stay registered on `AgentRegistry` with their own `Session` logs. Flushing only the prompted session after `when_idle` leaves those children without a directory, so settlement and resume cannot observe a child `{id}/session.jsonl`.

## Decision

`AgentRegistry::list` returns a snapshot of live `AgentHandle` values in unspecified order. After `when_idle` returns, `dsh-headless` `headless-runner` and `dsh-sdk-jsonrpc-server` `session/prompt` iterate `list()` and call `JsonlSessionStore::flush` on each live `Session`. Every still-registered session, including continuable children, writes `{DSH_SESSION_ROOT}/{id}/session.jsonl`. Children are not disposed at parent idle.

## Testing

`DSH_RUNTIME=rust` headless `subagent-settlement` asserts at least two directories under `{cwd}/.sessions`. `AgentRegistry::list` has no crate-local test.

## Alternatives considered

**Flush only the prompted session.** Rejected: continuable children never receive `{id}/session.jsonl` after `when_idle`.

**Flush on every session append.** Rejected: idle is the existing durability barrier; per-event flush adds I/O without a second consumer at process exit.

**Dispose children at parent idle, then flush survivors.** Rejected: continuable children must remain registered for `send_message` and settlement.

**Have each child flush itself when it becomes idle.** Rejected: the parent host already owns the process-exit persist; a second writer would race the same JSONL file.

## Consequences

A child removed from the registry before the parent's `when_idle` returns is not flushed here. The JSON-RPC server ignores individual flush errors (`let _ =`); headless returns the first flush error. Callers must not depend on `list` order.

## Related

The JSON-RPC host is [Rust SDK JSON-RPC server](2026-08-16-rust-sdk-jsonrpc-server.md). `list` is documented on [dsh-agent](../../../../crates/dsh-agent/README.md). Snapshot persist path and rust settlement coverage are [Rust snapshot harness](../../proposed/testing/2026-08-15-rust-snapshot-harness.md).
