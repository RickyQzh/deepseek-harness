# dsh-terminal-bash

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的交互式 bash PTY 后端。`register` 挂载 YAML `@deepseek-ai/dsh-terminal-bash`（`dsh_boot::PLUGIN_TERMINAL_BASH`），注入 `terminals`（`Mutex<TerminalSessionService>`）、`subprocess`（`LocalSubprocessRuntime`）和 `sandboxPolicy`（`SandboxPolicyResolver`），然后按配置的后端类型（默认 `shell`）注册。在 `danger-full-access` 下，spawn 的 argv 是 `[shellPath, ...shellArgs]`。任何其他已解析模式会在 spawn 时可选查找 `sandbox`（`LocalSandboxProvider`），并经 `confine` 包装。缺少 provider 时在 `spawn_terminal` 之前失败，文案为 `terminal-bash: sandbox mode "{mode}" requires a ctx.sandbox provider in the execution world`。本后端不依赖 `dsh-agent`、`dsh-acp`、`dsh-host`、`dsh-cli`、`dsh-headless`、`dsh-mcp-client` 或 `dsh-base`。本 crate 不挂入 `base.cordis.yml`。

未知配置键导致加载失败（`TerminalBashConfig: unknown key "{key}"`）。默认值：`backendType` 为 `shell`，`shellPath` 为 `/bin/bash`，`shellArgs` 为 `--noprofile --norc -i`，`rows` 为 40，`cols` 为 160，`scrollbackLines` 为 10000，`scrollbackMaxBytes` 为 4194304，`maxReadBytes` 为 262144，`pollIntervalMs` 为 50，`exactProbeAfterMs` 为 150，`idleSilenceMs` 为 3000，`handoffGraceMs` 为 500，`timeoutMs` 为 30000，`disposeGraceMs` 为 3000。空的 `backendType` 或 `shellPath` 分别以 `terminal-bash: backendType must be non-empty` / `terminal-bash: shellPath must be non-empty` 失败。非正数或非整数的数值字段以 `terminal-bash: {name} must be a positive safe integer` 失败。`maxReadBytes` 高于 `scrollbackMaxBytes` 时失败，文案为 `terminal-bash: maxReadBytes must not exceed scrollbackMaxBytes`。`handoffGraceMs` 小于 `pollIntervalMs` 时失败，文案为 `terminal-bash: handoffGraceMs must be at least pollIntervalMs so one readiness poll runs inside the grace window`。

在 subprocess 已清洗的父进程环境之后叠加的子进程环境为：`TERM=dumb`、`PAGER=cat`、`GIT_PAGER=cat`、`PS1=dsh> `（含尾随空格）、带上次退出码的 `PROMPT_COMMAND` OSC `133;D;`、`BASH_SILENCE_DEPRECATION_WARNING=1`、`DSH_SHELL=1`、`DSH_SESSION_ID=<owner>`、`DSH_PTY_SESSION_ID=<id>`。spawn 省略 `cwd` 时，默认工作目录是沙箱策略的 workspace root。

就绪检测使用私有 OSC `133;D;` 标记，随后可打印尾部须等于 `dsh> `；每次 send（包括 initialize 的空写入）都会丢弃写入前的证据；尚未发布的启动过程不接受零输出静默。在 `exactProbeAfterMs` 之后，当尚未发布的启动过程已有输出时，`inspect_foreground` 的 `input_waiting` 可以结算为 `stdin_read`。同一前台 PGID 必须先离开写入前的等待再重新进入；不同 PGID 可以使用当前等待。未知前台绝不是 `stdin_read`。提示符标记加上 `dsh> `、空闲至少 `pollIntervalMs`、且前台等于已捕获的 `shellPgid` 时，也会结算为 `stdin_read`。其他结算原因是 `inferred_idle`、`timeout` 和 `session_exit`。超时拒绝 spawn，文案为 `PTY shell did not reach readiness before startup timeout`。initialize 期间会话退出则为 `PTY shell exited during startup`。取消向前台进程组投递 `SIGINT`，从不写入 `\x03`。

## 模型体验

间接通过 terminal 工具消费方。本后端不贡献工具 schema 或提示词。

#### KV Cache 影响

无直接失效。

## 已知限制与延后工作

- 面向模型的 `terminal_*` 工具在 `dsh-tool-terminal`。
- Windows ConPTY 不在范围内。
- TUI 重写不在范围内。
- Headless `pty-tools` 仍留在 Node。
- JSON-RPC `persistent-tools` 仍留在 Node。
- 全屏 / alternate-screen PTY 不在范围内。
