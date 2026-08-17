# Agent Note: 冻结 Rust 持久 PTY 会话与具名 ACP pty-tools 场景

Status: proposed

[English](2026-08-17-rust-persistent-pty.md) | 中文

## 问题

[Rust 重写](2026-08-14-rust-rewrite.md)第 8 阶段第 3 项是 Terminal + PTY（仅 POSIX），经由 `portable-pty`。TypeScript 已经交付按 owner 隔离的持久会话、基于 `spawnTerminal` 的 bash 后端，以及六个面向模型的 `terminal_*` 工具。Rust 宿主只有管道 spawn。若没有冻结，移植可以跳过 Linux `/proc` stdin-wait（从而改变对模型可见的 `waitReason`）、把 PTY 工具挂到共享的 ACP（Agent Client Protocol）`rust.snapshot.cordis.yml` 上（破坏 handshake schema）、改写 TUI、启用 ConPTY、用 PTY 替换一次性 bash，或把 jsonrpc `persistent-tools` 当作第 8 阶段第 3 项。

## 提案

第 8 阶段第 3 项在 Rust 宿主上实现既有的持久 PTY 能力，YAML 配置项名为 `@deepseek-ai/dsh-terminal`、`@deepseek-ai/dsh-terminal-bash` 与 `@deepseek-ai/dsh-tool-terminal`。POSIX 分配在 `dsh-subprocess::spawn_terminal` 内使用 `portable-pty` 0.9。Linux 对 `/proc/<pid>/syscall` 与 `/proc/<pid>/mem` 的 stdin-wait 探测属于第 8 阶段第 3 项，以便 `waitReason` 可以为 `stdin_read`。Cordis、Landlock、`!!js`、会话格式、rusqlite 与 `native/landlock-run` 的保留或放弃引用[重写笔记](2026-08-14-rust-rewrite.md)。第 8 阶段第 3 项不增加 rusqlite、不移植 `!!js`、不改写 `landlock-run`、不改写 TUI，也不编辑 [docs/architecture.md](../../../../docs/architecture.md)。

本笔记并不取代[持久化 PTY 会话](../../implemented/feature/2026-07-16-persistent-pty-sessions.md)。那篇笔记仍是 TypeScript 约定的所有者。本笔记记录 Rust POSIX 子集与具名 Vitest 切换点。

所有权是 `ToolExecution` 上确切的 `SessionId`（每个会话一个 `LoopAgent`）。省略 `session_id` 或点名另一会话 id 的调用方失败关闭。注册表 id 从 1 起铸造 `pty-N`。取消向当前前台进程组投递真正的 `SIGINT`，并且从不写入 `\x03`。对顶层 shell 的 `SIGKILL` 以 TypeScript 句子拒绝。拆除时用启动身份围栏每一个捕获的 PID。

`register_base_plugins` 注册这三种插件类型，外加 YAML `pty-snapshot-backend`。默认的 headless、ACP、web 与 jsonrpc 组合不挂载 PTY 配置项，且不得分配 PTY。没有 `--profile pty`。

## 基底冻结

`portable-pty` 是 MIT。只在 `cfg(unix)` 下依赖它。非 unix 的 `spawn_terminal` 返回 `UnsupportedPlatform`，且不得调用 ConPTY。子进程环境从 `scrubbed_parent_env()` 起步，再应用终端专用 overlay，包括 `TERM=dumb` 与 `PS1=dsh> `。

Linux 检查器复制 TypeScript 的 `process-inspector.ts`：解析 `/proc/<pid>/stat`、x86_64 与 aarch64 的 syscall 表、用于 select/poll fd 集合的 `/proc/<pid>/mem`，以及 epoll fdinfo 的 `tfd: 0`。不可读的进程内存是 miss，绝不是肯定的 `stdin_read`。macOS 的 `isStdinWaiting` 为 false；就绪判定是提示符标记加静默。

具名 ACP `pty-tools` 使用内存中的 `pty-snapshot-backend`（MOTD 为 `dsh> `，send 回显 `text` 然后是 `PTY_OK`，`waitReason: stdin_read`），匹配 TypeScript 的 `pty.cordis.snapshot.yml`。真实 bash PTY 的覆盖是 `cargo test`。Rust ACP 启动器为该 overlay 加载 `rust.pty.snapshot.cordis.yml`，并让共享的 `rust.snapshot.cordis.yml` 不含 PTY 配置项。

## 第 8 阶段 PTY 子集

| 范围 | 本项 Rust | 留在 TypeScript / 后续 |
|---|---|---|
| ACP `pty-tools`（快照后端） | `DSH_RUNTIME=rust` 时的 Vitest | |
| 真实 bash PTY + Linux `stdin_read` | crate 测试 | |
| Headless `pty-tools` | 不在范围内 | Node（`.mjs` + include 插件） |
| jsonrpc `persistent-tools` | 不在范围内 | Node（`dsh-tool-bash-persistent` + editor） |
| TUI / Windows ConPTY / E2B | 不在范围内 | 后续 / 不在 v1 范围内 |

## 曾考虑的替代方案

**跳过 `/proc` 内存探测，并把每一次 Linux send 都结算为提示符或 `inferred_idle`。** 否决：重写笔记的风险一节写明，只等待提示符或静默的 portable-pty 移植会改变对模型可见的发送完成。第 8 阶段第 3 项保留第 1 层 `stdin_read`。

**把 PTY 工具挂到共享的 `examples/acp-agent/rust.snapshot.cordis.yml` 上。** 否决：handshake 与 `text-turn` 钉住的 schema 会多出六个工具。Overlay `rust.pty.snapshot.cordis.yml` 才是切换文件。

**用 PTY 替换一次性 `bash`。** 被 TypeScript PTY 笔记否决：一次性工具保有更强的校验、审批、沙箱、输出界限与回放约定。

**经由 napi `node-pty` 分配。** 否决：重写是一个 Rust 宿主。`portable-pty` 是点名的分配器。

**在第 8 阶段第 3 项移植 `@deepseek-ai/dsh-tool-bash-persistent` 与 `str_replace_editor`。** 否决：快照 harness 笔记已经把 jsonrpc `persistent-tools` 推迟到这两个工具都存在之后。

**把完整的 `examples/acp-agent` 与 headless `pty-tools` 当作切换点。** 否决：headless pty-tools 需要 include 插件和 `.mjs` 后端。具名 Rust 子集只有 ACP `pty-tools`。

## 验收标准

- 重写笔记的后续表链接到本文件。
- Linux `stdin_read` 保留 `/proc` stdin-wait；`portable-pty` 仅限 unix。
- 具名 Vitest ACP 子集增加 `pty-tools`；共享的 rust ACP YAML 不含 PTY 配置项；headless pty-tools 与 jsonrpc persistent-tools 留在 Node。
- 默认组合不分配 PTY；第 8 阶段第 3 项新增文件中没有 YAML `!!js`。
- 不编辑 [docs/architecture.md](../../../../docs/architecture.md)。
- 本笔记并不取代 TypeScript 持久 PTY 笔记。

## 风险

评审者可能因为 macOS 已经没有第 1 层，而把提示符/静默当作足够。Linux 的 `waitReason` 仍会改变。

评审者可能把 PTY 配置项加进共享的 `rust.snapshot.cordis.yml`。Handshake schema 会漂移。

评审者可能要求 Rust 上的 jsonrpc `persistent-tools`。该场景需要不同的消费方以及 editor 工具。
