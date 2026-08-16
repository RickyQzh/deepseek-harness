# dsh-shell

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的 shell 请求/规格类型、无围栏 POSIX bash 执行器，以及退出状态解析。

`ShellExecRequest` 是调用方面向的请求：`workdir`、`timeout_ms` 与 `stdout_max_bytes` 可以省略。`ShellExecSpec` 是上述字段已填入并封顶后的已解析形式。`LocalBashExecutor::resolve` 负责该默认化；`run` 与 `start` 接受规格，不再对请求中省略的字段重新默认。

`LocalBashExecutor` 通过 `dsh-subprocess` 把 `bash -c <command>` 作为受管进程组 spawn。`sandbox_mode()` 为 `None`；此执行器忽略 `sandbox_policy` 且不加约束。非零命令退出、超时与中止 kill 均解析为 `Ok(ShellRunResult)`，而非 `Err`。

`parse_exit_status` 是 shell 工具追加的 `[exit code: N]` / `[killed by signal: NAME]` 标记的逆解析。若文本以 kill 标记结尾，则得到 `signal` 并从 `body` 去掉该标记。否则若以退出码标记结尾，则得到 `exit_code` 并去掉该标记。否则 `body` 保持不变且 `exit_code` 为 `0`。超时与沙箱拒绝标记留在 `body` 中。

`resolve` 拒绝不以 `DSH_` 开头的 `dsh_env` 键。

## 已知限制与暂缓事项

- 无约束：`sandbox_mode()` 为 `None`，且忽略 `sandbox_policy`；稍后的沙箱执行器再包装 argv。
- 无持久 shell 或 PTY：每次调用都是全新的非 login `bash -c`。
- 仅 POSIX：`bash` 二进制写死，进程组语义也是 POSIX。
