# dsh-tools

English | [中文](README.zh.md)

Tool execution types, lossless-JSON argument freeze, and the pre-execute / approval / guard / execute / post-execute pipeline for the Rust host. Bash and filesystem tools do not live here.

`ToolRuntime::execute` runs that order. Under `ToolPresentationMode::Code`, a model-direct call whose name is registered and is not `run_code` is denied before pre-execute (collapse-before-policy). Nested calls (`parent` set) and unknown names skip collapse: unknown names still run pre-execute, then fail as `unknown tool "{name}"`.

`freeze_args` clones a `serde_json::Value` so policy listeners receive a detached copy. `freeze_args_from_raw` maps an empty model string to `{}` and keeps invalid JSON as a string, matching the TypeScript loop's `parseArguments`. Product call ids are `dsh_session::CallId`.

## Known Limitations and Deferred Work

- Scoped restrictions, `presentAs`, and the `run_code` worker are later phases. This crate implements collapse-before-policy only.
