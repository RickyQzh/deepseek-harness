# dsh-shell

English | [中文](README.zh.md)

Shell request/spec types, an unfenced POSIX bash executor, and exit-status parse for the DeepSeek Harness Rust host.

`ShellExecRequest` is the caller-facing request: `workdir`, `timeout_ms`, and `stdout_max_bytes` may be omitted. `ShellExecSpec` is the resolved form with those fields filled and capped. `LocalBashExecutor::resolve` performs that defaulting; `run` and `start` take a spec and never re-default omitted request fields.

`LocalBashExecutor` spawns `bash -c <command>` as a managed process group through `dsh-subprocess`. `sandbox_mode()` is `None`; this executor ignores `sandbox_policy` and does not confine. Nonzero command exits, timeouts, and abort kills resolve as `Ok(ShellRunResult)`, not `Err`.

`parse_exit_status` is the inverse of the `[exit code: N]` / `[killed by signal: NAME]` markers shell tools append. A trailing kill marker yields `signal` and strips the marker from `body`. Else a trailing exit-code marker yields `exit_code` and strips it. Otherwise `body` is unchanged and `exit_code` is `0`. Timeout and sandbox-denial markers stay in `body`.

`resolve` rejects `dsh_env` keys that do not start with `DSH_`.

## Known Limitations and Deferred Work

- Unconfined: `sandbox_mode()` is `None` and `sandbox_policy` is ignored; a later sandboxing executor wraps argv.
- No persistent shell or PTY: every call is a fresh non-login `bash -c`.
- POSIX-only: the `bash` binary is hardcoded and process-group semantics are POSIX.
