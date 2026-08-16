# Agent Note: Rust agent-instructions baseline inject and JSONL resume

Status: implemented

English | [中文](2026-08-16-rust-agent-instructions-baseline.zh.md)

## Problem

The TypeScript [workspace-context plugin](../feature/2026-06-24-workspace-context.md) injects `AGENTS.md` / `CLAUDE.md` as a logged `user/message` with `source.kind == "agent-instructions"`. Phase 5 of the [Rust rewrite](../../proposed/architecture/2026-08-14-rust-rewrite.md) needs that baseline on the Rust host so headless resume can rematch after an offline file edit, without mounting the plugin in `register_spine_plugins`, and without a `dsh-agent-loop` → `dsh-agent-instructions` crate cycle.

TypeScript `workspaceBaselineIdentity` hashes discovery config only. File-content drift is detected later by `state.ts` reconciliation. This crate has no `state.rs`, so a config-only identity would treat an offline `AGENTS.md` edit as a match and skip the new baseline.

## Decision

`dsh-agent-instructions` registers YAML name `@deepseek-ai/dsh-agent-instructions` (`PLUGIN_AGENT_INSTRUCTIONS` in `dsh-boot` only). `maxBytes` is required; unknown keys fail load. Discovery walks from the session `cwd` up through `projectRootMarkers` (default `.git`), loads every existing candidate per directory (base then `.local`), and `$DSH_HOME/AGENTS.md` with display path `$DSH_HOME/AGENTS.md`. When `fs` is provided, reads go through `LocalFileSystem::read_text`; unit tests may use `std::fs`.

An `agent/pre-step` listener reads the live session through `CompactionScope` (synchronous `with_session` only). On `Enter { messages }` with a non-empty vec, it prepends a user-role `MessageSource::AgentInstructions` (`form: "instructions"`, `baseline: true`) when the visible surface has no such baseline or `baselineIdentity` mismatches, then always calls `next(decision)`. Empty `Enter` is left unchanged so an instructions-only step is not invented. Wrapper strings are copied from TypeScript `render.ts`: `<system-reminder>` frame, `WORKSPACE_CONTEXT_INTRO`, and `Instructions from: {displayPath}\n\n{content}`. Escape replaces `</system-reminder>` with `<\/system-reminder>`.

`baselineIdentity` is a JSON digest of discovery config **and** `{path, digest}` per loaded file (SHA-1 of file bytes), so an offline edit mismatches without reconciliation. `MessageSource::AgentInstructions` serializes `kind: "agent-instructions"` via existing `rename_all = "kebab-case"`. `SESSION_FORMAT_VERSION` stays `0`.

`JsonlSessionStore::load` reads `path_for(id)`, `decode_session_log`, then `Session::from_events`. `AgentRegistry::resume` requires `options.session_id == session.id()`, returns the live handle when that id is already registered, otherwise builds `LoopAgent::new` from the loaded session (not `Session::new`). Headless YAML `resumeSessionId` loads then resumes, then still followups the cmdline task.

## Alternatives considered

**Port TypeScript config-only `workspaceBaselineIdentity`.** Rejected: without `state.ts` reconciliation, an offline `AGENTS.md` edit would keep the same identity and skip inject. File digests are the resume check this crate owns.

**Insert the baseline after claimed messages, as TypeScript does.** Rejected for this phase: `Enter { messages }` is the log, and the binding inject rule is prepend then `next()`. Claimed user text still enters; it is not dropped.

**Add `Session::from_replay` beside `from_events`.** Rejected: `Session::from_events` already rebuilds the surface.

**Register from `register_spine_plugins`.** Rejected: spine composition stays the closed list in [Spine YAML plugins](2026-08-16-spine-plugins-in-dsh-agent.md).

**Omit this plugin from default Phase 6 YAML.** Rejected: `base.cordis.yml` mounts `@deepseek-ai/dsh-agent-instructions` with `maxBytes: 65536` through [dsh-base](2026-08-16-rust-dsh-base-plugins.md).

**Let `dsh-agent-loop` depend on this crate.** Rejected: the loop owns the waterfall type; this crate is a listener. The reverse edge would cycle once the loop is in the spine graph.

## Consequences

Products that want workspace instructions list `@deepseek-ai/dsh-agent-instructions` with an explicit `maxBytes`. Dynamic reconciliation after filesystem tool touches, nested descendant discovery, and the full TypeScript omit/truncate renderer remain later work. `cargo test -p dsh-agent-instructions --offline` pins first-request inject and resume-after-offline-edit. `cargo test -p dsh-session-persist --offline load_round_trips` and `cargo test -p dsh-headless --offline resume` pin JSONL load and YAML `resumeSessionId`.

## Related

Product behavior of the TypeScript plugin is [workspace context](../feature/2026-06-24-workspace-context.md), [load-all dedup](../feature/2026-07-21-instruction-load-all-dedup.md), and [local overlay](../feature/2026-07-21-local-instruction-overlay.md).
