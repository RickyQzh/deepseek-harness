# dsh-shell

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的 shell 请求/规格类型、POSIX bash 执行器，以及退出状态解析。

`ShellExecRequest` 是调用方面向的请求：`workdir`、`timeout_ms` 与 `stdout_max_bytes` 可以省略。`ShellExecSpec` 是上述字段已填入并封顶后的已解析形式。`LocalBashExecutor::resolve` 与 `SandboxBashExecutor::resolve` 负责该默认化；`run` 与 `start` 接受规格，不再对请求中省略的字段重新默认。

`LocalBashExecutor` 通过 `dsh-subprocess` 把 `bash -c <command>` 作为受管进程组 spawn。`sandbox_mode()` 为 `None`；此执行器忽略 `sandbox_policy` 且不加约束。非零命令退出、超时与中止 kill 均解析为 `Ok(ShellRunResult)`，而非 `Err`。

`SandboxBashExecutor` 通过 `Confine` 实现（`LocalSandboxProvider` 实现 `Confine`）包装 `LocalBashExecutor`。`resolve` 把 `sandbox_policy` 盖章为请求值或构造默认值；`resolve` 之后规格上的策略始终为 `Some`。在此执行器上，`danger-full-access` 是唯一不加约束的路径：`run`/`start` 调用内层执行器且永不 `confine`。任何其他模式都包装 `["bash", "-c", command]` 并失败即关闭——`confine` 错误或已分类的 runner 失败是 `SANDBOX_UNAVAILABLE`，且不把命令视为已运行。Landlock 启动器失败是退出码 125 加上一行致命的 `landlock-run: ` stderr（排除整行部分强制执行通知之后）；仅有 125 不足以判定启动器失败。

`parse_exit_status` 是 shell 工具追加的 `[exit code: N]` / `[killed by signal: NAME]` 标记的逆解析。若文本以 kill 标记结尾，则得到 `signal` 并从 `body` 去掉该标记。否则若以退出码标记结尾，则得到 `exit_code` 并去掉该标记。否则 `body` 保持不变且 `exit_code` 为 `0`。超时与沙箱拒绝标记留在 `body` 中。

`resolve` 拒绝不以 `DSH_` 开头的 `dsh_env` 键。

`plugin::register` 挂载 YAML `@deepseek-ai/dsh-shell-bash-local`，注入 `subprocess`，并提供无围栏的 `shell` 服务（`LocalBashExecutor`）。

## 已知限制与暂缓事项

- `LocalBashExecutor` 仍无约束：`sandbox_mode()` 为 `None`，且忽略 `sandbox_policy`。
- 后台 `ShellProcess` 没有沙箱事实；把 runner 失败分类为 `SANDBOX_UNAVAILABLE` 仅适用于前台 `run`。
- 无持久 shell 或 PTY：每次调用都是全新的非 login `bash -c`。
- 仅 POSIX：`bash` 二进制写死，进程组语义也是 POSIX。
