# dsh-tool-fs

English | [中文](README.zh.md)

Model-facing `read`, `write`, and `edit` filesystem tools for the DeepSeek Harness Rust host.

`register_fs_tools` registers those three names on a `ToolRuntime`. `search` is not registered here. `FsToolContext` holds the local filesystem, observation gate and owner, optional sandbox policy, a subprocess runtime (for later `search`), and `rg_binary`.

`read` resolves `file_path`, stats, observes absence on miss (`FS_NOT_FOUND`), rejects non-files (`FS_NOT_REGULAR_FILE`), then reads UTF-8 text, observes presence, and returns `{ path, text }`. Render is 1-based numbered lines `{line_no:>6}|{line}`; a trailing newline yields a last empty segment.

`write` and `edit` take mutation intent from `ObservationGate` before calling `write_text` / `edit_text`. Edit without a prior read for that owner is `FS_NOT_OBSERVED`. Guarded-mutation messages gain ` — re-read the file, then retry` (`FS_STALE_VERSION`) or ` — read the file, then retry` (`FS_NOT_OBSERVED`) through `remediate_fs_error`. `FsError` becomes `ToolError::Coded` with name `FsError` and the `FS_*` code.

Escalation fields `sandbox_permissions` and `justification` are refused when `FsToolContext.sandbox` is `None` or the filesystem is unfenced: `sandbox_permissions is not available in this composition (no sandboxing filesystem to escalate)`.

## Known Limitations and Deferred Work

- `search` is not registered; `rg_binary` and the subprocess runtime are stored for a later crate change.
- Read does not window, stream, or cap lines; it returns the whole UTF-8 file as numbered lines.
- `read_image` is not registered.
- Escalation pairing is validated, but approved wider-mode retries are not implemented; standing `ctx.sandbox` is passed through to mutations.
