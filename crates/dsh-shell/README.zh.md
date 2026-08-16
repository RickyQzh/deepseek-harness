# dsh-shell

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的 shell 请求/规格类型与退出状态解析。

`ShellExecRequest` 是调用方面向的请求：`workdir`、`timeout_ms` 与 `stdout_max_bytes` 可以省略。`ShellExecSpec` 是上述字段已填入并封顶后的已解析形式。执行器的 `resolve` 负责该默认化；`run` 与 `start` 接受规格，不再对请求中省略的字段重新默认。本 crate 不实现执行器。

`parse_exit_status` 是 shell 工具追加的 `[exit code: N]` / `[killed by signal: NAME]` 标记的逆解析。若文本以 kill 标记结尾，则得到 `signal` 并从 `body` 去掉该标记。否则若以退出码标记结尾，则得到 `exit_code` 并去掉该标记。否则 `body` 保持不变且 `exit_code` 为 `0`。超时与沙箱拒绝标记留在 `body` 中。

`dsh_env` 的键必须以 `DSH_` 开头。本 crate 不校验该前缀。

## 已知限制与暂缓事项

- 执行器（`resolve` / `run` / `start`）不在本 crate；`ShellProcess` 尚无方法。
- `dsh_env` 的键不对照 `DSH_` 前缀检查。
