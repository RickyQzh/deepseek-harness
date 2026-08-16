# dsh-fs

English | [中文](README.zh.md)

Filesystem types, `FS_*` error codes, and the unfenced local UTF-8 backend for the DeepSeek Harness Rust host.

`FsTargetKey` and `FsVersion` are local newtypes around `dsh_brand::Branded`. Branding does not trim or validate. `FsError` carries a stable `FsErrorCode`; `as_str` returns the TypeScript wire string.

`FS_SANDBOX_DENIED` is an in-process fence refusal. `FS_PERMISSION_DENIED` is a kernel or OS permission failure. They are not interchangeable.

`LocalFileSystem` joins relative paths against `cwd` (or a per-call cwd), realpaths the deepest existing ancestor plus remaining suffix, and uses that canonical string for both `target_key` and `display_path`. `read_text` accepts regular UTF-8 files, rejects NUL in the first 8192 bytes and invalid UTF-8, and normalizes `\r\n` to `\n`. `write_text` publishes through a `0o600` sibling temp file then `rename`; omit the intent for unconditional create-or-overwrite. `edit_text` applies a literal replace on LF-normalized text and restores CRLF when that was the original style. Version tokens are `{dev}:{ino}:{size}:{mtime_nsec}:{ctime_nsec}`. `sandbox_mode()` returns `None`. `sandbox_policy` arguments are ignored.

## Known Limitations and Deferred Work

- No sandbox fence or observation policy; `sandbox_mode()` is `None` and `sandbox_policy` is ignored until Task 37.
