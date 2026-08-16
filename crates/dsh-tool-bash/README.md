# dsh-tool-bash

English | [中文](README.zh.md)

Model-facing `bash` tool for the DeepSeek Harness Rust host.

`register_bash_tool` registers the name `bash` on a `ToolRuntime`. When `sandbox_shell` is `Some`, execute uses `SandboxBashExecutor`; otherwise it uses `LocalBashExecutor`. The tool is exclusive (`is_concurrency_safe` is `None`).

Calls are foreground only: `resolve` then `run`. `run_in_background: true` is `ToolError::Other` with `Background execution is not available; long-running commands must finish within the timeout.` The tool description includes that sentence. Jobs, `start`, system-prompt sections, and escalation approval are not implemented.

`command` and `description` must be strings whose trim is nonempty. `timeout_ms`, when present, must be a positive finite number. `workdir` and `timeout_ms` are optional request overlays; other `ShellExecRequest` fields stay `None` except `signal`, which is the tool-call abort flag.

`render_result` builds the model-visible text: stdout, then `[stderr]\n{stderr}` when stderr is nonempty, or `(no output)` when both streams are empty. Truncated streams append `\n[output truncated; full output: {path|'(unavailable)'}]`. Markers then follow in order: sandbox denial (`[sandbox: file access denied under {mode} mode]`) plus the shared escalation hint when `escalation_modes` is nonempty, `[timed out after {n}ms]`, `[killed by signal: SIG…]`, or `[exit code: N]` for a nonzero exit. A clean exit 0 has no exit marker.

Nonzero bash exits are successful tool results. `SANDBOX_UNAVAILABLE` from `ShellError::Sandbox` is `ToolError::Coded` with name `SandboxUnavailableError` and code `SANDBOX_UNAVAILABLE`. Abort after the body is `AbortError` / `ABORTED`. `sandbox_permissions` and `justification` are refused in every composition: `sandbox_permissions is not available in this composition (no sandboxing executor to escalate)`.

## Known Limitations and Deferred Work

- Background jobs, `run_in_background` schema advertisement, and `job_output` / `job_kill` are not implemented.
- Escalation approval is not implemented; `sandbox_permissions` is always refused, including when a sandbox executor is mounted.
- The tool does not contribute a `tool:bash` system-prompt section or UI `presentCall` / `presentResult`.
- Per-session cwd and `DSH_*` overlays are not applied; omitted `workdir` uses the executor default.
