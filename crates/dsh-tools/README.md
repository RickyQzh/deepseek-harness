# dsh-tools

English | [中文](README.zh.md)

Tool execution types and lossless-JSON argument freeze for the Rust host. The pipeline (pre-execute, approval, guards, execute, post-execute, finalize) and Code Mode collapse live in this crate; bash and filesystem tools do not.

`freeze_args` clones a `serde_json::Value` so policy listeners receive a detached copy. `freeze_args_from_raw` maps an empty model string to `{}` and keeps invalid JSON as a string, matching the TypeScript loop's `parseArguments`. Product call ids are `dsh_session::CallId`.

## Known Limitations and Deferred Work

- Scoped restrictions, `presentAs`, and the `run_code` worker are later phases. This crate implements collapse-before-policy only.
