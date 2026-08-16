# dsh-subprocess

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的完全指定 argv spawn 与凭据擦除后的子进程环境。`argv` 绝不是 shell 字符串；需要 shell 的消费方自行传入 `["bash", "-c", command]`。

子进程环境先擦除再叠加合并：`child_env` 从 `scrubbed_parent_env()` 开始（丢弃环境中形似凭据的名称与 `DSH_*` 名称），再应用显式 `EnvEntry`。`None` 是移除环境键的 tombstone；POSIX 下后出现的精确键胜出。

## 已知限制与暂缓事项

- 本 crate 不含 PTY 分配。
- POSIX 进程组属于 Task 32。
- `argv` 绝不是 shell 字符串。
- Windows `taskkill` 不在本 crate。
