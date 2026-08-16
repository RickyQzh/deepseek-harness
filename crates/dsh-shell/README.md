# dsh-shell

English | [中文](README.zh.md)

Shell request/spec types and exit-status parse for the DeepSeek Harness Rust host.

`ShellExecRequest` is the caller-facing request: `workdir`, `timeout_ms`, and `stdout_max_bytes` may be omitted. `ShellExecSpec` is the resolved form with those fields filled and capped. Executor `resolve` performs that defaulting; `run` and `start` take a spec and never re-default omitted request fields. This crate does not implement the executor.

`parse_exit_status` is the inverse of the `[exit code: N]` / `[killed by signal: NAME]` markers shell tools append. A trailing kill marker yields `signal` and strips the marker from `body`. Else a trailing exit-code marker yields `exit_code` and strips it. Otherwise `body` is unchanged and `exit_code` is `0`. Timeout and sandbox-denial markers stay in `body`.

`dsh_env` keys must start with `DSH_`. This crate does not validate that prefix.

## Known Limitations and Deferred Work

- The executor (`resolve` / `run` / `start`) is not in this crate; `ShellProcess` has no methods yet.
- `dsh_env` keys are not checked against the `DSH_` prefix.
