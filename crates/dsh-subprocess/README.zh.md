# dsh-subprocess

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的完全指定 argv spawn 与凭据擦除后的子进程环境。`argv` 绝不是 shell 字符串；需要 shell 的消费方自行传入 `["bash", "-c", command]`。

子进程环境先擦除再叠加合并：`child_env` 从 `scrubbed_parent_env()` 开始（丢弃环境中形似凭据的名称与 `DSH_*` 名称），再应用显式 `EnvEntry`。`None` 是移除环境键的 tombstone；POSIX 下后出现的精确键胜出。

`spawn_subprocess` 会启动 POSIX 进程组组长（`process_group(0)`），按上限收集输出尾部（可选 spill 文件），并以向 `-pid` 发送 SIGTERM、在 `grace_ms` 后再发送 SIGKILL 的方式终止。`LocalSubprocessRuntime` 在擦除后的 `PATH` 上解析裸名称，并在 dispose 时终止每棵仍存活的进程树且等待 `wait_for_exit`。

## 已知限制与暂缓事项

- 本 crate 不含 PTY 分配。
- Windows `taskkill /T` 进程树终止暂缓；非 unix 上 `spawn_subprocess` 返回 `UnsupportedPlatform`。
- `argv` 绝不是 shell 字符串。
