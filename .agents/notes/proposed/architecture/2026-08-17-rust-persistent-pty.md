# Agent Note: Freeze the Rust persistent PTY sessions and named ACP pty-tools scenario

Status: proposed

English | [中文](2026-08-17-rust-persistent-pty.zh.md)

## Problem

The [Rust rewrite](2026-08-14-rust-rewrite.md) Phase 8 third item is Terminal + PTY (POSIX only) via `portable-pty`. TypeScript already ships owner-scoped persistent sessions, a bash backend over `spawnTerminal`, and six model-facing `terminal_*` tools. The Rust host has pipe spawn only. Without a freeze, a port can skip Linux `/proc` stdin-wait (changing model-visible `waitReason`), mount PTY tools on the shared ACP `rust.snapshot.cordis.yml` (breaking handshake schemas), rewrite the TUI, enable ConPTY, replace one-shot bash, or treat jsonrpc `persistent-tools` as Phase 8 item 3.

## Proposal

Phase 8 item 3 implements the existing persistent PTY capability on the Rust host as YAML rows named `@deepseek-ai/dsh-terminal`, `@deepseek-ai/dsh-terminal-bash`, and `@deepseek-ai/dsh-tool-terminal`. POSIX allocation uses `portable-pty` 0.9 inside `dsh-subprocess::spawn_terminal`. Linux stdin-wait probing of `/proc/<pid>/syscall` and `/proc/<pid>/mem` is in Phase 8 item 3 so `waitReason` can be `stdin_read`. Cite the [rewrite note](2026-08-14-rust-rewrite.md) for keep-or-drop of Cordis, Landlock, `!!js`, session format, rusqlite, and `native/landlock-run`. Phase 8 item 3 does not add rusqlite, does not port `!!js`, does not rewrite `landlock-run`, does not rewrite the TUI, and does not edit [docs/architecture.md](../../../../docs/architecture.md).

This note does not supersede [persistent PTY sessions](../../implemented/feature/2026-07-16-persistent-pty-sessions.md). That note remains the TypeScript contract owner. This note records the Rust POSIX subset and the named Vitest cutover.

Ownership is the exact `SessionId` on `ToolExecution` (one `LoopAgent` per session). A caller that omits `session_id` or names another session's id fails closed. Registry ids mint `pty-N` from 1. Cancel delivers a real `SIGINT` to the current foreground process group and never writes `\x03`. `SIGKILL` of the top-level shell is refused with the TypeScript sentence. Teardown fences every captured PID with start identity.

`register_base_plugins` registers the three plugin types plus YAML `pty-snapshot-backend`. Default headless, ACP, web, and jsonrpc compositions do not mount PTY rows and must not allocate a PTY. There is no `--profile pty`.

## Substrate freeze

`portable-pty` is MIT. Depend on it only under `cfg(unix)`. Non-unix `spawn_terminal` returns `UnsupportedPlatform` and must not call ConPTY. Child env starts from `scrubbed_parent_env()` and then applies terminal-specific overlays including `TERM=dumb` and `PS1=dsh> `.

The Linux inspector copies TypeScript `process-inspector.ts`: parse `/proc/<pid>/stat`, x86_64 and aarch64 syscall tables, `/proc/<pid>/mem` for select/poll fd sets, and epoll fdinfo `tfd: 0`. Unreadable process memory is a miss, never a positive `stdin_read`. macOS `isStdinWaiting` is false; readiness is prompt-marker plus silence.

Named ACP `pty-tools` uses the in-memory `pty-snapshot-backend` (MOTD `dsh> `, send echoes `text` then `PTY_OK`, `waitReason: stdin_read`), matching TypeScript `pty.cordis.snapshot.yml`. Real bash PTY coverage is `cargo test`. The rust ACP launcher loads `rust.pty.snapshot.cordis.yml` for that overlay and leaves shared `rust.snapshot.cordis.yml` without PTY rows.

## Phase 8 PTY subset

| Surface | Rust this item | Stays TypeScript / later |
|---|---|---|
| ACP `pty-tools` (snapshot backend) | Vitest when `DSH_RUNTIME=rust` | |
| Real bash PTY + Linux `stdin_read` | crate tests | |
| Headless `pty-tools` | out | Node (`.mjs` + include plugin) |
| jsonrpc `persistent-tools` | out | Node (`dsh-tool-bash-persistent` + editor) |
| TUI / Windows ConPTY / E2B | out | later / out of v1 |

## Alternatives considered

**Skip `/proc` memory probes and settle every Linux send as prompt or `inferred_idle`.** Rejected: the rewrite risk section states that a portable-pty port which only waits for prompt or silence changes model-visible send completion. Phase 8 item 3 keeps Tier 1 `stdin_read`.

**Mount PTY tools on shared `examples/acp-agent/rust.snapshot.cordis.yml`.** Rejected: handshake and `text-turn` schema pins would gain six tools. Overlay `rust.pty.snapshot.cordis.yml` is the cutover file.

**Replace one-shot `bash` with PTY.** Rejected by the TypeScript PTY note: one-shot tools keep stronger validation, approval, sandbox, output-bound, and replay contracts.

**Allocate through napi `node-pty`.** Rejected: the rewrite is one Rust host. `portable-pty` is the named allocator.

**Port `@deepseek-ai/dsh-tool-bash-persistent` and `str_replace_editor` in Phase 8 item 3.** Rejected: the snapshot-harness note already defers jsonrpc `persistent-tools` until both of those tools exist.

**Treat full `examples/acp-agent` and headless `pty-tools` as the cutover.** Rejected: headless pty-tools needs the include plugin and a `.mjs` backend. Named rust subset is ACP `pty-tools` only.

## Acceptance criteria

- The rewrite note follow-up table links to this file.
- Linux `stdin_read` keeps `/proc` stdin-wait; `portable-pty` is unix-only.
- Named Vitest ACP subset gains `pty-tools`; shared rust ACP YAML stays without PTY rows; headless pty-tools and jsonrpc persistent-tools stay Node.
- Default compositions do not allocate a PTY; YAML `!!js` is absent from files Phase 8 item 3 adds.
- [docs/architecture.md](../../../../docs/architecture.md) is not edited.
- This note does not supersede the TypeScript persistent PTY note.

## Risks

A reviewer may treat prompt/silence as enough because macOS already lacks Tier 1. Linux `waitReason` would still change.

A reviewer may add PTY rows to shared `rust.snapshot.cordis.yml`. Handshake schemas would drift.

A reviewer may demand jsonrpc `persistent-tools` on Rust. That scenario needs a different consumer and the editor tool.
