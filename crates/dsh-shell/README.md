# dsh-shell

English | [中文](README.zh.md)

Shell request/spec types, POSIX bash executors, and exit-status parse for the DeepSeek Harness Rust host.

`ShellExecRequest` is the caller-facing request: `workdir`, `timeout_ms`, and `stdout_max_bytes` may be omitted. `ShellExecSpec` is the resolved form with those fields filled and capped. `LocalBashExecutor::resolve` and `SandboxBashExecutor::resolve` perform that defaulting; `run` and `start` take a spec and never re-default omitted request fields.

`LocalBashExecutor` spawns `bash -c <command>` as a managed process group through `dsh-subprocess`. `sandbox_mode()` is `None`; this executor ignores `sandbox_policy` and does not confine. Nonzero command exits, timeouts, and abort kills resolve as `Ok(ShellRunResult)`, not `Err`.

`SandboxBashExecutor` wraps a `LocalBashExecutor` through a `Confine` implementation (`LocalSandboxProvider` implements `Confine`). `resolve` stamps `sandbox_policy` from the request or the constructor default; after `resolve` the spec's policy is always `Some`. `danger-full-access` is the only unconfined path on this executor: `run`/`start` call the inner executor and never `confine`. Any other mode wraps `["bash", "-c", command]` and fails closed — a `confine` error or classified runner failure is `SANDBOX_UNAVAILABLE`, and the command is not treated as having run. Landlock launcher failure is exit 125 plus a fatal `landlock-run: ` stderr line after excluding the whole-line partial-enforcement notice; 125 alone is not a launcher failure.

`parse_exit_status` is the inverse of the `[exit code: N]` / `[killed by signal: NAME]` markers shell tools append. A trailing kill marker yields `signal` and strips the marker from `body`. Else a trailing exit-code marker yields `exit_code` and strips it. Otherwise `body` is unchanged and `exit_code` is `0`. Timeout and sandbox-denial markers stay in `body`.

`resolve` rejects `dsh_env` keys that do not start with `DSH_`.

`plugin::register` mounts YAML `@deepseek-ai/dsh-shell-bash-local`, injects `subprocess`, and provides unfenced `shell` as `LocalBashExecutor`.

## Known Limitations and Deferred Work

- `LocalBashExecutor` stays unconfined: `sandbox_mode()` is `None` and `sandbox_policy` is ignored.
- Background `ShellProcess` has no sandbox facts; runner-failure classification to `SANDBOX_UNAVAILABLE` is foreground `run` only.
- No persistent shell or PTY: every call is a fresh non-login `bash -c`.
- POSIX-only: the `bash` binary is hardcoded and process-group semantics are POSIX.
