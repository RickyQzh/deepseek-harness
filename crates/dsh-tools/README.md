# dsh-tools

English | [中文](README.zh.md)

Tool execution types, lossless-JSON argument freeze, and the pre-execute / approval / guard / execute / post-execute pipeline for the Rust host. Bash and filesystem tools do not live here.

`ToolRuntime::execute` runs that order by composing `prepare`, `dispatch`, and `finalize`. `begin_prepare` assigns a real token and runs freeze, collapse, and abort-before-policy on `&mut self` without `.await`. `PrepareSnapshot::finish` then runs pre-execute, the optional `Approver`, and guards; `prepare` is that pair without a session. `dispatch` returns a `'static` future for `definition.execute` plus render; that future does not borrow the runtime, so overlapping bodies can join while the next `prepare` uses `&mut self`. `finalize` runs post-execute. A `prepare` deny or abort-before-body never reaches `dispatch`. Under `ToolPresentationMode::Code`, a model-direct call whose name is registered and is not `run_code` is denied before pre-execute (collapse-before-policy). Nested calls (`parent` set) and unknown names skip collapse: unknown names still run pre-execute, then fail as `unknown tool "{name}"`. `ToolError::Coded` Display is the message only; execute puts `{ name, code }` on `error.info` and content `Error: {message}`.

`freeze_args` clones a `serde_json::Value` so policy listeners receive a detached copy. `freeze_args_from_raw` maps an empty model string to `{}` and keeps invalid JSON as a string, matching the TypeScript loop's `parseArguments`. Product call ids are `dsh_session::CallId`. `ToolRuntime` `Clone` copies registered tools, pre/post listeners, mode, and `next_token`, and drops Box guards and the `Approver`. Ask with no `Approver` denies with `tool "{name}" requires approval, but no approval channel is available`. `set_approver` installs the session-aware decision path; `set_approval` wraps a session-free hook.

`plugin::register` provides `tools` as an empty native `ToolRuntime`. `registered_names` returns the sorted model-facing names currently registered.

## Known Limitations and Deferred Work

- Scoped restrictions, `presentAs`, and the `run_code` worker are later phases. This crate implements collapse-before-policy only.
