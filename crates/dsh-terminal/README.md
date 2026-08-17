# dsh-terminal

English | [中文](README.zh.md)

Owner-scoped persistent PTY registry for the DeepSeek Harness Rust host. `register` mounts YAML `@deepseek-ai/dsh-terminal` (`dsh_boot::PLUGIN_TERMINAL`) and provides `terminals` as `Mutex<TerminalSessionService>`. `register_snapshot_backend` mounts YAML `pty-snapshot-backend` (`dsh_boot::PLUGIN_PTY_SNAPSHOT_BACKEND`), injects `terminals`, and registers backend type `shell`. `register_terminal_plugins` installs terminal first so the snapshot backend can inject. This crate does not allocate a real PTY and does not depend on `dsh-agent` or `dsh-subprocess`. The real bash PTY backend is `dsh-terminal-bash`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

`TerminalSessionId` is a local newtype around `dsh_brand::Branded<TerminalSessionIdTag>`. `new` is crate-private; callers mint ids through `spawn`, which issues `pty-1`, `pty-2`, … . Owner is `dsh_session::SessionId`. `has_owner_activity` is true from an unpublished spawn reservation through close, with no publication gap. `start_send` is exclusive (`SEND_ACTIVE`, `PTY session {id} already has an active send`). Missing id is `NO_SESSION` (`unknown PTY session {id}`). Foreign owner is `FOREIGN_SESSION` and the message includes the id. Empty backend type fails; a duplicate type is `DUPLICATE_BACKEND` (`a PTY backend named "{type}" is already registered`); a missing type on spawn is `NO_BACKEND` (`no PTY backend registered for "{type}"`).

The in-memory snapshot backend MOTD is `dsh> ` (trailing space). A send of text `hi` settles with viewport `hi\nPTY_OK\ndsh> `, `waitReason` `stdin_read`, and `sessionStatus` running. Snapshot `cancel()` returns false because `done` is already resolved.

PLUGIN_TERMINAL accepts no config keys; unknown keys fail load. PLUGIN_PTY_SNAPSHOT_BACKEND likewise rejects unknown keys. Neither plugin spawns a PTY at load.

## Model Experience

Indirectly through terminal tool consumers. This registry contributes no tool schema or prompt.

#### KV Cache effect

No direct invalidation.

## Known Limitations and Deferred Work

- The crate ships an in-memory snapshot backend only; real bash PTY allocation lives in `dsh-terminal-bash`.
- Sessions are process-local and are not restored after a harness restart.
- The plugins are not mounted in `base.cordis.yml` in this phase.
