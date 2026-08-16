# dsh-tool-bash

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的模型侧 `bash` 工具。

`register_bash_tool` 在 `ToolRuntime` 上注册名称 `bash`。当 `sandbox_shell` 为 `Some` 时，execute 使用 `SandboxBashExecutor`；否则使用 `LocalBashExecutor`。该工具是互斥的（`is_concurrency_safe` 为 `None`）。

调用仅前台运行：先 `resolve` 再 `run`。`run_in_background: true` 为 `ToolError::Other`，消息为 `Background execution is not available; long-running commands must finish within the timeout.` 工具描述包含该句。不实现后台任务、`start`、系统提示词段落，以及提权批准。

`command` 与 `description` 必须是 trim 后非空的字符串。若存在 `timeout_ms`，必须是正有限数。`workdir` 与 `timeout_ms` 是可选的请求覆盖；其余 `ShellExecRequest` 字段保持 `None`，但 `signal` 除外，它是此次工具调用的中止标志。

`render_result` 生成模型可见文本：先 stdout，stderr 非空时再接 `[stderr]\n{stderr}`，两路皆空则为 `(no output)`。被截断的流追加 `\n[output truncated; full output: {path|'(unavailable)'}]`。随后按顺序追加标记：沙箱拒绝（`[sandbox: file access denied under {mode} mode]`），并在 `escalation_modes` 非空时加上共享提权提示；然后是 `[timed out after {n}ms]`、`[killed by signal: SIG…]`，或非零退出的 `[exit code: N]`。干净的退出码 0 不附加退出标记。

非零 bash 退出仍是成功的工具结果。来自 `ShellError::Sandbox` 的 `SANDBOX_UNAVAILABLE` 变为名为 `SandboxUnavailableError`、码为 `SANDBOX_UNAVAILABLE` 的 `ToolError::Coded`。主体之后的中止是 `AbortError` / `ABORTED`。任何组合都拒绝 `sandbox_permissions` 与 `justification`：`sandbox_permissions is not available in this composition (no sandboxing executor to escalate)`。任何组合下 `escalation_modes` 均为空，因此沙箱拒绝不会追加同轮次重试提示。

## 已知限制与暂缓事项

- 不实现后台任务、`run_in_background` 的 schema 宣告，以及 `job_output` / `job_kill`。
- 不实现提权批准；即使挂载了沙箱执行器，也始终拒绝 `sandbox_permissions`。拒绝结果不宣告同轮次提权。
- 该工具不贡献 `tool:bash` 系统提示词段落，也不提供 UI 的 `presentCall` / `presentResult`。
- 不应用按会话的 cwd 与 `DSH_*` 覆盖；省略 `workdir` 时使用执行器默认值。
