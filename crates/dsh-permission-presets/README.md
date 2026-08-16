# dsh-permission-presets

English | [中文](README.zh.md)

Pins `permission/preset`, `sandbox/mode`, and `approval/policy` into each new session so later default changes cannot alter an existing log. `PermissionPresetService::pin_initial` writes all three knobs on a fresh unseeded session (`seed_length` absent and no `session/end-seed`); a seeded or partial log keeps present knobs and receives only the missing ones.

YAML `@deepseek-ai/dsh-permission-presets` config is `{ presets?, defaultPreset? }`. Omitting `presets` loads `workspace-write` (`workspace-write` + `ask`) and `danger-full-access` (`danger-full-access` + `never`); `read-only` is present only when YAML lists it. Omitting `defaultPreset` selects the table row matching composed sandbox and approval; no match is a load error, not `custom`. An unknown `defaultPreset` fails with `unknown preset`. A table key `custom` is reserved. `plugin::register` lives in this crate; `dsh-boot` does not call it.

The plugin injects `agents`, `approval`, and `shell`, provides `permissionPresets`, and registers `agents.on_session_create`. Composed sandbox is `LocalBashExecutor::sandbox_mode()`, or `workspace-write` when that is `None`. Composed approval is `ApprovalService`'s default (`ask` unless YAML `policy: never`). See the [permission pin invariant](../../.agents/notes/proposed/architecture/2026-08-14-rust-rewrite.md).

## Known Limitations and Deferred Work

- The `/permission` command and `permissions` projection are not in this crate.
- `defaultPreset` is YAML only; there is no settings-file default for later sessions.
- `LocalBashExecutor::sandbox_mode` is always `None`, so composed sandbox is `workspace-write` until a confining executor is provided.
