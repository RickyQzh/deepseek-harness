# dsh-subprocess

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的完全指定 argv spawn 与凭据擦除后的子进程环境。`argv` 绝不是 shell 字符串；需要 shell 的消费方自行传入 `["bash", "-c", command]`。

子进程环境先擦除再叠加合并：`child_env` 从 `scrubbed_parent_env()` 开始（丢弃环境中形似凭据的名称与 `DSH_*` 名称），再应用显式 `EnvEntry`。`None` 是移除环境键的 tombstone；POSIX 下后出现的精确键胜出。

`spawn_subprocess` 会启动 POSIX 进程组组长（`process_group(0)`），按上限收集输出尾部（可选 spill 文件），并以向 `-pid` 发送 SIGTERM、在 `grace_ms` 后再发送 SIGKILL 的方式终止。直接子进程退出后，collect 模式的 `done()` 最多等待 `grace_ms` 以读到管道 EOF，随后丢弃 collect 读端，因此被后代继承的描述符无法挂起结算。`LocalSubprocessRuntime` 在擦除后的 `PATH` 上解析裸名称，并在 dispose 时终止每棵仍存活的进程树且等待 `wait_for_exit`。

`spawn_terminal` 用 `portable-pty` 打开 POSIX PTY（`PtySize` 的 rows/cols，像素尺寸为 0），按 cwd 与 `env_clear` 之后的 `child_env` spawn `argv`，向 master 写入字节，并在 `tokio::sync::broadcast` 通道上发布 UTF-8 有损 `String` 分块。`done` 在顶层 PTY 子进程退出时结算。最后一个 handle drop 会关闭 master 并向子进程发信号。`LocalSubprocessRuntime::spawn_terminal` 会保留 clone，直到 `dispose` 丢弃它们；`dispose` 不等待 PTY 子进程。

`plugin::register` 挂载 YAML `@deepseek-ai/dsh-subprocess-local`，并提供 `subprocess` 服务（`LocalSubprocessRuntime`）。

## 已知限制与暂缓事项

- Windows ConPTY 不在范围内；非 unix 上 `spawn_terminal` 返回 `UnsupportedPlatform`。
- PTY 的 inspect、signal 与 terminate 尚未实现；`dispose` 不等待 PTY 子进程。
- Windows `taskkill /T` 进程树终止暂缓；非 unix 上 `spawn_subprocess` 返回 `UnsupportedPlatform`。
- `argv` 绝不是 shell 字符串。
