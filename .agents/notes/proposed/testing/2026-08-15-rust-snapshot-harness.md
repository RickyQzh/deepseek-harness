# Agent Note: Keep Vitest snapshot drivers; spawn the Rust bin for Phase 5 scenarios

Status: proposed

English | [中文](2026-08-15-rust-snapshot-harness.zh.md)

## Problem

Phase 5 of the [Rust rewrite](../architecture/2026-08-14-rust-rewrite.md) makes the first shipping host a Rust binary (`dsh` headless and `dsh-jsonrpc-agent`). [testing.md](../../../../docs/testing.md) requires keyless snapshots through a real assembled example, and the [tooling note](../process/2026-08-14-rust-tooling-and-gates.md) requires those snapshots to run the built binary once a bin exists. The rewrite follow-up table left a placeholder for this decision. Dual-run can go false-green if snapshots stay on the TypeScript bin after the product bin is Rust, or if both bins run different compositions as the merge gate.

## Proposal

Keep the existing Vitest snapshot drivers in `examples/jsonrpc-agent/tests/sdk.snapshot.ts` and `examples/headless-agent`. Do not port the driver to `cargo test` and do not duplicate fixture directories.

When `DSH_RUNTIME=rust`, the jsonrpc driver spawns `target/debug/dsh-jsonrpc-agent` (or `DSH_RUNTIME_BIN` when set) instead of the Node `dsh-jsonrpc-agent` source launch. The same `examples/jsonrpc-agent/tests/snapshots/<name>/` directories remain the replay corpus: the driver still writes `DSH_SNAPSHOT_FILE` from `session.jsonl`. Node remains the default when `DSH_RUNTIME` is unset.

Phase 6 jsonrpc scenarios on the Rust bin are `text-turn`, `bash-tool`, and `subagent-spawn-in-process`. `persistent-tools` stays on the Node driver until a later phase that ships persistent shell and `str_replace_editor`.

Phase 6 headless scenarios on the Rust `dsh` bin are `compaction-recovery`, `provider-retry`, `agent-instructions` resume (`workspace-context-resume.snapshot.ts`), and `subagent-settlement`. `pty-tools`, `ralph-loop`, `goal-tools`, `advanced-toolchain`, and `headless-profile` stay on the Node bin.

When `DSH_RUNTIME=rust`, the headless driver spawns `target/debug/dsh` (or `DSH_RUNTIME_BIN` when set) with `--profile headless` and the positional task. Static YAML is `DSH_CORDIS_CONFIG` pointing at `examples/headless-agent/rust.<scenario>.cordis.yml` (no `!!js`, no include plugin). Persist path stays `{DSH_SESSION_ROOT}/{sessionId}/session.jsonl`.

On `DSH_RUNTIME=rust`, assert scenario-specific durable facts plus process exit 0 and last assistant / stdout. Skip full equality against Node-recorded `stream-json.expected.jsonl` when the Rust composition omits session-title LLM or other `dsh-base` rows this phase does not port. The Node default keeps full equality. Do not re-record fixtures unless a frozen wire field is wrong.

Do not re-record `notifications.expected.jsonl`, `result.expected.json`, or fixture `session.jsonl` unless a real wire mismatch against the frozen SDK JSON-RPC methods is proven. A composition mismatch (no session-title LLM, no permission presets, no compaction, no `dsh-base` tools) is expected for the Phase 5 minimal YAML and is not a reason to rewrite Node fixtures. On `DSH_RUNTIME=rust`, assert `result.expected.json` `finalResponse`, the last `session.status` notification `idle`, persist path `$DSH_SESSION_ROOT/<sessionId>/session.jsonl`, and initialize `serverInfo.name = deepseek-harness-sdk-runtime`. Skip full equality against Node-recorded notification JSONL on that launch path. The Node default keeps full equality.

Cite the rewrite note for keep-or-drop of Cordis, Landlock, `!!js`, and session format rather than restating those rows here.

Phase 7 named web scenarios on the Rust `dsh` bin are `rust-host-smoke` and `cold-blank-session`. Remaining `test:web` files stay on the Node scaffold or jsdom. Full `pnpm run test:web` against Rust is the rewrite-program exit named in the [rewrite note](../architecture/2026-08-14-rust-rewrite.md), not the Phase 7 cutover.

When `DSH_RUNTIME=rust`, those named web drivers spawn `target/debug/dsh` (or `DSH_RUNTIME_BIN`) with argv `web --port 0 --dist <dir>`. Unset `DSH_RUNTIME` keeps the in-process Cordis scaffold. `built-boot.snapshot.ts` stays jsdom/`FixtureApiClient` (no host).

Phase 8 item 1 named ACP scenarios on the Rust `dsh` bin are `handshake`, `reject-extra-dirs`, and `text-turn`. Full `pnpm run test:snapshot` against Rust is not Phase 8 item 1's exit.

When `DSH_RUNTIME=rust`, those named ACP drivers spawn `target/debug/dsh` (or `DSH_RUNTIME_BIN`) with `--profile acp`. Unset `DSH_RUNTIME` keeps the Node `dsh-acp-demo` bin.

## Phase 6 subset

| Scenario | Driver | Bin | Fixture dir |
|---|---|---|---|
| jsonrpc `text-turn` | Vitest `sdk.snapshot.ts` | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/jsonrpc-agent/tests/snapshots/text-turn/` |
| jsonrpc `bash-tool` | Vitest `sdk.snapshot.ts` | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/jsonrpc-agent/tests/snapshots/bash-tool/` |
| jsonrpc `subagent-spawn-in-process` | Vitest `sdk.snapshot.ts` | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/jsonrpc-agent/tests/snapshots/subagent-spawn-in-process/` |
| jsonrpc `persistent-tools` | Vitest `sdk.snapshot.ts` | Node only | `examples/jsonrpc-agent/tests/snapshots/persistent-tools/` |
| headless `compaction-recovery` | Vitest `headless.snapshot.ts` | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/headless-agent/tests/snapshots/compaction-recovery/` |
| headless `provider-retry` | Vitest `headless.snapshot.ts` | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/headless-agent/tests/snapshots/provider-retry/` |
| headless agent-instructions resume | Vitest `workspace-context-resume.snapshot.ts` | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/headless-agent/tests/workspace-context-resume-snapshots/` |
| headless `subagent-settlement` | Vitest `headless.snapshot.ts` | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/headless-agent/tests/snapshots/subagent-settlement/` |
| headless pty/ralph/goal/advanced/headless-profile | existing Vitest | Node only | existing dirs |

## Phase 7 subset

| Scenario | Driver | Bin | Fixture dir |
|---|---|---|---|
| web `rust-host-smoke` | Vitest `apps/web/tests/rust-host-smoke.e2e.ts` | Rust when `DSH_RUNTIME=rust`; skipped otherwise | none |
| web `cold-blank-session` | Vitest `cold-blank-session.e2e.ts` | Rust when `DSH_RUNTIME=rust`; Node scaffold otherwise | `apps/web/tests/snapshots/cold-blank-session/` |
| remaining `test:web` files | existing Vitest | Node scaffold / jsdom | existing dirs |

## Phase 8 ACP subset

| Scenario | Driver | Bin | Fixture dir |
|---|---|---|---|
| ACP `handshake` | Vitest `examples/acp-agent/tests/acp.snapshot.ts` | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/acp-agent/tests/snapshots/handshake/` |
| ACP `reject-extra-dirs` | same | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/acp-agent/tests/snapshots/reject-extra-dirs/` |
| ACP `text-turn` | same | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/acp-agent/tests/snapshots/text-turn/` |
| ACP `pty-tools` | same | Rust when `DSH_RUNTIME=rust`; Node otherwise | `examples/acp-agent/tests/snapshots/pty-tools/` |
| remaining ACP scenarios | same suite | Node only | existing dirs |

`DSH_RUNTIME=rust` must not drop scenarios from the table (orphan-dir guard) and must skip non-subset **runs**. Remaining ACP scenarios are every name not listed as Rust in this table. This note does not claim full `pnpm run test:snapshot` on Rust.

## Phase 8 MCP subset

Phase 8 item 2 does not add named Vitest snapshot scenarios. TypeScript `@deepseek-ai/dsh-mcp-client` already chose unit/e2e coverage and no snapshots, because an MCP row would mutate pinned system-prompt fixtures and spawn an external server. Rust coverage is `cargo test -p dsh-mcp-client` against an in-crate Content-Length stdio fixture. `DSH_RUNTIME=rust` snapshot drivers must not mount `@deepseek-ai/dsh-mcp-client`.

## Phase 8 PTY subset

Phase 8 item 3 names ACP `pty-tools` as a Rust scenario when `DSH_RUNTIME=rust`. The rust ACP launcher loads `examples/acp-agent/rust.pty.snapshot.cordis.yml` when the Node overlay basename is `pty.cordis.yml` (sibling `rust.<stem>.snapshot.cordis.yml`). Shared `rust.snapshot.cordis.yml` stays without PTY rows so handshake and `text-turn` schemas stay stable. Coverage for real bash PTY and Linux `stdin_read` is `cargo test`. Headless `pty-tools` and jsonrpc `persistent-tools` stay on the Node bin.

## Alternatives considered

**Rewrite snapshot drivers in Rust (`cargo test` spawning nothing, or a Rust NDJSON client).** Rejected: the product test is the assembled application transcript; a second suite against a different composition is the dual-run failure the rewrite note names. Vitest already owns normalization, `llm-replay` hydration, and expected-output comparison.

**Re-record every jsonrpc expected JSONL against the Phase 5 minimal bin.** Rejected unless a method name, `serverInfo.name`, or `session.event` envelope field on the frozen wire is wrong. Composition-thinner logs are an accepted Phase 5 gap, not a fixture rewrite.

**Drop Vitest and treat crate tests as the snapshot gate.** Rejected by the tooling note: once a bin exists, snapshots that claim the shipping product must run that binary.

**Spawn the Rust bin for every existing headless and jsonrpc scenario in Phase 5.** Rejected for pty/ralph/goal/advanced: those scenarios require Phase 8 capabilities. Phase 6 names four headless scenarios plus jsonrpc `subagent-spawn-in-process` as the cutover. Node remains the driver for the rest.

**Spawn the Rust bin for every `test:web` file in Phase 7.** Rejected: named subset is `rust-host-smoke` and `cold-blank-session`. Remaining files stay Node. Full `pnpm run test:web` against Rust is the rewrite-program exit.

**Spawn the Rust bin for every `examples/acp-agent` scenario in Phase 8 item 1.** Rejected: named subset is `handshake`, `reject-extra-dirs`, and `text-turn`. Remaining ACP scenarios stay Node. Full `pnpm run test:snapshot` against Rust is not Phase 8 item 1's exit.

**Put PTY tools on shared `rust.snapshot.cordis.yml`.** Rejected: handshake and `text-turn` schema pins would gain six tools.

## Acceptance criteria

- The rewrite note follow-up table links to this file instead of the placeholder ``proposed/testing/…-rust-snapshot-harness.md``.
- `DSH_RUNTIME=rust` is documented as the Vitest launch switch; unset keeps Node.
- Phase 6 names the four headless scenarios and jsonrpc `subagent-spawn-in-process` as the Rust subset.
- Fixture directories are reused; the plan does not add parallel `*.rust.expected.jsonl` files.
- Phase 7 names web `rust-host-smoke` and `cold-blank-session` as the Rust subset; remaining `test:web` files stay Node. This note does not claim full `pnpm run test:web` on Rust.
- Phase 8 item 1 names ACP `handshake`, `reject-extra-dirs`, and `text-turn` as the Rust subset. This note does not claim full `pnpm run test:snapshot` on Rust.
- Phase 8 item 3 names ACP `pty-tools` as a Rust scenario when `DSH_RUNTIME=rust`; remaining ACP scenarios stay Node; shared rust ACP YAML stays without PTY rows; headless pty-tools and jsonrpc persistent-tools stay Node.
- `docs/architecture.md` is not edited.

## Risks

A reviewer may treat skipped Node `stream-json.expected.jsonl` and notification-JSONL equality on the Rust path as weakening the snapshot gate. The Node default still pins the full transcript; the Rust path pins scenario-specific durable facts, process exit 0, last assistant / stdout, and jsonrpc `finalResponse`, idle status, persist path, and `serverInfo.name`. Phase 6 composition does not match the Node event stream.

A reviewer may treat the named web subset as full `pnpm run test:web` on Rust. Remaining web e2e stay Node.

A reviewer may treat the named ACP subset as full `pnpm run test:snapshot` on Rust. Remaining ACP scenarios stay Node.
