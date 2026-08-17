# dsh-workspace

English | [中文](README.zh.md)

Durable JSON workspace registry for the DeepSeek Harness Rust host. A workspace is a stable id over an existing directory, a display title, and the ordered session account. This crate does not serve HTTP and does not read session headers.

`WorkspaceRegistry::with_path` persists one camelCase JSON object at that path (`workspaceIds`, `archivedSessionIds`, `workspaces`). A missing file is an empty registry. Corrupt JSON fails the next `list` or mutation. Writes use a same-directory temp file plus rename. In-process callers are serialized with a `Mutex`.

`create` requires an existing directory (`std::fs::canonicalize`); it does not mkdir. The same canonical path returns that workspace with `created: false` and an unchanged title. A new workspace is prepended; the default title is the path basename. Distinct canonical paths may share a display title at create. Ids are `wk-{pid}-{nanos}`. Timestamps are ISO-8601 UTC (`YYYY-MM-DDTHH:MM:SS.sssZ`).

`rename` trims; empty is `title-invalid`; another workspace with the same title is `workspace-name-conflict`; renaming to the current title is a no-op. `delete` unregisters only (directory and session logs stay); an unknown id is `workspace-not-found`. `insert_before` is DOM-insertBefore (omitted anchor appends). `attach_session` prepends when absent and is otherwise idempotent. `archive_session` appends to the registry-global archive set and leaves `sessionIds` unchanged.

`plugin::register` mounts YAML `@deepseek-ai/dsh-workspace` and provides `workspaces`. Config is optional `{ "path": "<string>" }`. When `path` is omitted, `{DSH_HOME}/workspaces.json` if `DSH_HOME` is set, else `{DSH_SESSION_ROOT}/workspaces.json`. Load fails if neither env is set and config omits `path`, or if the persist file's parent cannot be created. Tests use `with_path`.

`WorkspaceError::code` is the kebab-case wire string; `rpc_code` maps to `dsh_rpc::RpcErrorCode`. This crate does not emit `session-not-found` or `workspace-attach-failed`.

## Known Limitations and Deferred Work

- Unary `workspace.*` RPC and host plugin wiring live outside this crate; `dsh-base` and `dsh-cli` do not register this plugin.
- Attach does not read session headers or depend on `dsh-session-persist`; cwd validation is a later host task.
