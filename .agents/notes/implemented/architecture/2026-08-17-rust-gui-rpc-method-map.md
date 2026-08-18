# Agent Note: Phase 7 GUI RpcMethodMap and slash remotes in dsh-host

Status: implemented

English | [中文](2026-08-17-rust-gui-rpc-method-map.zh.md)

## Problem

The Phase 7 Rust GUI host must answer the remaining dotted `RpcMethodMap` methods and the two Typert slash remotes the TypeScript SPA already calls, without composing `web.cordis.yml` or depending on `dsh-subagent`. Leaving those names as carrier HTTP 404 would blank workspace create, settings describe, skill list, and slash command discovery on loopback.

## Decision

`dsh-host` mounts `GuiHandler` beside `SessionHandler`. `spawn_stub_host` stays `StubHandler`. HTTP tests of the combined map listen through `spawn_gui_host` / `listen_with_handler`.

Dispatch after the `/api` trust fence is JSON Content-Type and `client-request` parse, then `method` equals the path suffix after `/api/`, then privileged re-check on dotted names only (`is_privileged_method` / `privileged_requires_loopback`). If that suffix contains `/`, the slash interceptor runs; otherwise `accepts_dotted`. Dedicated `POST /api/respond` and `GET`/`HEAD` `/api/events.mux` and `/api/events.host` never enter the interceptor.

Slash remotes are only `commands/list` and `commands/execute` with payload `{ args }`. List returns `{ commands: [{ name, description }] }` (empty array is success). Execute runs `CommandRegistry::parse` then `execute`; a miss is `RpcResult::err` `unknown-command` at HTTP 200. Any other `/api/foo/bar`, including `goals/create`, is HTTP 404.

Dotted `GuiHandler` reuses `WorkspaceRegistry`, `SettingsService`, `LayeredCredentials`, `CommandRegistry`, `SkillRegistry::list`, and `SessionHandler`. `host.describe` fills `attachedSessions` from `AgentRegistry::list().len()` and optional `provider`/`model` from the first LLM provider or lookup defaults. `host.listDirectory` lists name-sorted directories only, cap 500 then `truncated: true`. `host.pickDirectory` is `directory-picker-unavailable`; `host.openPath` is `internal` and `canOpenPath` stays false. Dotted `goal.*` is `internal` `"goals are not implemented in Phase 7"`. `agentPreset.list` is the read-only `standard` roster; authoring methods are `agent-preset-read-only`. `subagent.list` returns `{ items: [] }` and does not invent ACP. `dsh-agent` does not depend on `dsh-host` or `dsh-subagent`; `dsh-host` does not depend on `dsh-subagent`.

The wire freeze, including slash-namespace 404 and the privileged set, remains in [Freeze the Rust GUI host four-quadrant wire](../../proposed/architecture/2026-08-16-rust-gui-host-wire.md).

## Testing

`workspace_create_via_http`, `skill_list_returns_name_and_description`, `agent_preset_list_is_standard_readonly`, `settings_describe_loopback_exposes_ui_onboarding`, `slash_commands_list_empty_array`, `slash_goals_create_is_http_404`, and `privileged_settings_describe_trusted_non_loopback_is_403` pin the map. `cargo test -p dsh-host --offline` keeps the unary, WebSocket, and session tests.

## Alternatives considered

**Point `spawn_stub_host` at `GuiHandler`.** Rejected: carrier tests must keep `StubHandler` so `host.describe` stays the only installed dotted name on that listener.

**Port every Typert slash remote.** Rejected by the wire freeze: Phase 7 installs `commands/list` and `commands/execute` only; unknown slash namespaces stay HTTP 404.

**Add `dsh-subagent` to `dsh-host` or `dsh-host` to `dsh-agent`.** Rejected: `subagent.list` returns an empty catalog; ACP is not invented here.

**Stub slash `goals/create` as dotted `goal.create` success.** Rejected: slash `goals/create` is not installed (HTTP 404); dotted `goal.*` answers `internal`.

## Consequences

Uninstalled slash namespaces degrade the TypeScript UI instead of dual-running a fake host. Privileged `settings.describe` from a trusted non-loopback Host stays HTTP 403. The bundled `web.cordis.yml` graph is owned by [the web plugin graph note](2026-08-17-rust-web-plugin-graph.md); `dsh-cli web` remains a later Phase 7 task.
