# dsh-fs

English | [中文](README.zh.md)

Filesystem types, `FS_*` error codes, the local UTF-8 backend, an in-process sandbox fence, and a per-owner observation map for the DeepSeek Harness Rust host.

`FsTargetKey` and `FsVersion` are local newtypes around `dsh_brand::Branded`. Branding does not trim or validate. `FsError` carries a stable `FsErrorCode`; `as_str` returns the TypeScript wire string.

`FS_SANDBOX_DENIED` is an in-process fence refusal. `FS_PERMISSION_DENIED` is a kernel or OS permission failure. They are not interchangeable.

`LocalFileSystem` joins relative paths against `cwd` (or a per-call cwd), realpaths the deepest existing ancestor plus remaining suffix, and uses that canonical string for both `target_key` and `display_path`. `read_text` accepts regular UTF-8 files, rejects NUL in the first 8192 bytes and invalid UTF-8, and normalizes `\r\n` to `\n`. `write_text` publishes through a `0o600` sibling temp file then `rename`; omit the intent for unconditional create-or-overwrite. `edit_text` applies a literal replace on LF-normalized text and restores CRLF when that was the original style. Version tokens are `{dev}:{ino}:{size}:{mtime_nsec}:{ctime_nsec}`.

`LocalFileSystem::new` is unfenced: `sandbox_mode()` is `None` and `sandbox_policy` is ignored. `LocalFileSystem::sandboxed` installs a `SandboxFence`; `sandbox_mode()` is that policy's mode. The fence is in-process containment, not a kernel boundary. `read-only` denies mutations with `FS_SANDBOX_DENIED`. `workspace-write` re-resolves `display_path` and requires `is_path_under` some `dsh_sandbox::writable_roots` entry, then mutates the fresh target. `danger-full-access` returns the original target. Reads are never fenced. `is_path_under` uses a case-sensitive lexical prefix with `MAIN_SEPARATOR` on Linux; when spellings differ it walks ancestors comparing `(dev, ino)`; a missing root is not contained.

`ObservationGate` records per-owner presence or absence keyed by `(owner id, target_key)` and derives write/edit intents from that map. Distinct owners do not share observations. A missing owner never looks up. The gate is not wired into `LocalFileSystem` methods.

`plugin::register` mounts YAML `@deepseek-ai/dsh-fs-local` and provides `fs` as `LocalFileSystem`. Cwd is config `cwd`, else `DSH_CWD`, else the process cwd.

## Known Limitations and Deferred Work

- The sandbox fence is in-process containment, not kernel isolation; a residual TOCTOU remains between re-resolve and the write syscall.
- `ObservationGate` is a standalone per-owner map; `LocalFileSystem` does not record observations on read, write, or edit.
