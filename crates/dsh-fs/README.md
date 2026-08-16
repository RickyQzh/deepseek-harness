# dsh-fs

English | [中文](README.zh.md)

Filesystem types and `FS_*` error codes for the DeepSeek Harness Rust host.

`FsTargetKey` and `FsVersion` are local newtypes around `dsh_brand::Branded`. Branding does not trim or validate. `FsError` carries a stable `FsErrorCode`; `as_str` returns the TypeScript wire string.

`FS_SANDBOX_DENIED` is an in-process fence refusal. `FS_PERMISSION_DENIED` is a kernel or OS permission failure. They are not interchangeable.

## Known Limitations and Deferred Work

- Local backend (Task 36), sandbox fence, and observation policy (Task 37) are not in this crate.
