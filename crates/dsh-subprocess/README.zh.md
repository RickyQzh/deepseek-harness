# dsh-subprocess

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的完全指定 argv spawn 与凭据擦除后的子进程环境。`argv` 绝不是 shell 字符串；需要 shell 的消费方自行传入 `["bash", "-c", command]`。

子进程环境先擦除再叠加合并：`child_env` 从 `scrubbed_parent_env()` 开始（丢弃环境中形似凭据的名称与 `DSH_*` 名称），再应用显式 `EnvEntry`。`None` 是移除环境键的 tombstone；POSIX 下后出现的精确键胜出。

`spawn_subprocess` 会启动 POSIX 进程组组长（`process_group(0)`），按上限收集输出尾部（可选 spill 文件），并以向 `-pid` 发送 SIGTERM、在 `grace_ms` 后再发送 SIGKILL 的方式终止。`Pipe` 的 stdin 与 stdout 处置方式通过一次性的 `take_stdin` 与 `take_stdout` 返回仍存活的 `ChildStdin` 与 `ChildStdout`；非 Pipe 处置方式返回 `None`。直接子进程退出后，collect 模式的 `done()` 最多等待 `grace_ms` 以读到管道 EOF，随后丢弃 collect 读端，因此被后代继承的描述符无法挂起结算。`LocalSubprocessRuntime` 在擦除后的 `PATH` 上解析裸名称，并在 dispose 时终止每棵仍存活的进程树且等待 `wait_for_exit`。

`spawn_terminal` 用 `portable-pty` 打开 POSIX PTY（`PtySize` 的 rows/cols，像素尺寸为 0），按 cwd 与 `env_clear` 之后的 `child_env` spawn `argv`，挂上 `create_process_inspector`，向 master 写入字节，并在 `tokio::sync::broadcast` 通道上发布 UTF-8 有损 `String` 分块。`done` 在顶层 PTY 子进程退出时结算。`inspect_foreground`、`signal_foreground` 与 `terminate` 是 `SubprocessTerminalHandle` 上的方法。PGID 缺失时 `inspect_foreground` 返回 `Ok(None)`。`signal_foreground` 通过 inspector 向进程组发送真实信号（从不向 PTY 写入 `\x03`），并在对 shell pid 发送 `SIGKILL` 时以 `refusing to SIGKILL the terminal shell; terminate the terminal session instead` 拒绝。`terminate` 复制 TypeScript `closeOnce`：按 PID+启动身份围栏快照后代、SIGTERM、等待 `grace_ms`（25 ms 轮询）、对幸存者 SIGKILL，再对 shell 发送 SIGTERM/SIGKILL；Linux zombie 视为已停稳。最后一个 handle drop 会关闭 master；若 `done` 已记录退出或已运行 `terminate`，则不再向子进程发信号。`LocalSubprocessRuntime::spawn_terminal` 会保留 clone，直到 `dispose` 对每个 handle 等待 `terminate`。

`create_process_inspector` 通过 Linux `/proc`（x86_64 与 aarch64 系统调用号表）和 macOS `ps` 检查前台 PGID、stdin 等待、子进程优先的进程树，以及 PID+启动身份。`read(0)` 视为 stdin 等待；无法读取的 `/proc/<pid>/mem` 不视为等待。macOS 上 `is_stdin_waiting` 恒为 false。

`plugin::register` 挂载 YAML `@deepseek-ai/dsh-subprocess-local`，并提供 `subprocess` 服务（`LocalSubprocessRuntime`）。

## 已知限制与暂缓事项

- Windows ConPTY 不在范围内；非 unix 上 `spawn_terminal` 返回 `UnsupportedPlatform`。
- 同步宿主退出路径 `terminateForHostExit` 未移植；最后一个 handle 的 Drop 仍用于防泄漏，并在 `done` 已记录或已运行 `terminate` 后跳过 kill。
- Windows `taskkill /T` 进程树终止暂缓；非 unix 上 `spawn_subprocess` 返回 `UnsupportedPlatform`。
- `argv` 绝不是 shell 字符串。
