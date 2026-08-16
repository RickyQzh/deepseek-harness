# dsh-tool-fs

English | [中文](README.zh.md)

Model-facing `read`, `write`, `edit`, `glob`, and `grep` filesystem tools for the DeepSeek Harness Rust host.

`register_fs_tools` registers those five names on a `ToolRuntime`. `FsToolContext` holds the local filesystem, observation gate and owner, optional sandbox policy, a subprocess runtime, and `rg_binary` (the `rg` argv[0]; never read from the environment).

`plugin::register` mounts YAML `@deepseek-ai/dsh-tool-fs`, injects `tools`, `fs`, and `subprocess`, and registers those five names on the shared `ToolRuntime` with `ObservationOwner(1)` and no sandbox.

`read` resolves `file_path`, stats, observes absence on miss (`FS_NOT_FOUND`), rejects non-files (`FS_NOT_REGULAR_FILE`), then reads UTF-8 text, observes presence, and returns `{ path, text }`. Render is 1-based numbered lines `{line_no:>6}|{line}`; a trailing newline yields a last empty segment.

`write` and `edit` take mutation intent from `ObservationGate` before calling `write_text` / `edit_text`. Edit without a prior read for that owner is `FS_NOT_OBSERVED`. Guarded-mutation messages gain ` — re-read the file, then retry` (`FS_STALE_VERSION`) or ` — read the file, then retry` (`FS_NOT_OBSERVED`) through `remediate_fs_error`. `FsError` becomes `ToolError::Coded` with name `FsError` and the `FS_*` code.

Escalation fields `sandbox_permissions` and `justification` are refused when `FsToolContext.sandbox` is `None` or the filesystem is unfenced: `sandbox_permissions is not available in this composition (no sandboxing filesystem to escalate)`.

`glob` and `grep` spawn `ctx.rg_binary` through `ctx.subprocess`. `--no-config` is the first argument after the binary (`argv[1]`), always, including when the rest of the argv is empty. The spawn never reads `RIPGREP_CONFIG_PATH`. Exit 0 or 1 is success (1 means no matches). Other exits become `ToolError::Coded` with name `SearchError`: `SEARCH_INVALID_PATTERN` when stderr matches `regex parse error` or `error parsing glob` (case-insensitive), `SEARCH_RAW_OUTPUT_OVERFLOW` when stdout collection is lossy, `SEARCH_ABORTED` when the abort flag is set, otherwise `SEARCH_FAILED`.

## Known Limitations and Deferred Work

- `glob`/`grep` use `ctx.rg_binary` rather than a packaged ripgrep; a missing binary fails the call as `SEARCH_FAILED`.
- Read does not window, stream, or cap lines; it returns the whole UTF-8 file as numbered lines.
- `read_image` is not registered.
- Escalation pairing is validated, but approved wider-mode retries are not implemented; standing `ctx.sandbox` is passed through to mutations.
