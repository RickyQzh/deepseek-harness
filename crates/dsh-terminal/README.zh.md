# dsh-terminal

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主、按 owner 隔离的持久 PTY 注册表。`register` 挂载 YAML `@deepseek-ai/dsh-terminal`（`dsh_boot::PLUGIN_TERMINAL`），并以 `Mutex<TerminalSessionService>` 提供 `terminals`。`register_snapshot_backend` 挂载 YAML `pty-snapshot-backend`（`dsh_boot::PLUGIN_PTY_SNAPSHOT_BACKEND`），注入 `terminals`，并注册后端类型 `shell`。`register_terminal_plugins` 先安装 terminal，以便 snapshot 后端能够注入。本 crate 不分配真实 PTY，也不依赖 `dsh-agent` 或 `dsh-subprocess`。真实 bash PTY 后端是 `dsh-terminal-bash`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

`TerminalSessionId` 是包住 `dsh_brand::Branded<TerminalSessionIdTag>` 的本地 newtype。`new` 为 crate 私有；调用方通过 `spawn` 铸造 id，依次发出 `pty-1`、`pty-2`、…。Owner 是 `dsh_session::SessionId`。`has_owner_activity` 从尚未发布的 spawn 预留一直为 true，直到 close，没有发布空隙。`start_send` 互斥（`SEND_ACTIVE`，`PTY session {id} already has an active send`）。缺失 id 为 `NO_SESSION`（`unknown PTY session {id}`）。他人 owner 为 `FOREIGN_SESSION`，消息包含该 id。空的后端类型会失败；重复类型为 `DUPLICATE_BACKEND`（`a PTY backend named "{type}" is already registered`）；spawn 时缺少类型为 `NO_BACKEND`（`no PTY backend registered for "{type}"`）。

内存 snapshot 后端的 MOTD 是 `dsh> `（含尾随空格）。发送文本 `hi` 时，viewport 结算为 `hi\nPTY_OK\ndsh> `，`waitReason` 为 `stdin_read`，`sessionStatus` 为 running。snapshot 的 `cancel()` 返回 false，因为 `done` 已经结算。

PLUGIN_TERMINAL 不接受任何配置键；未知键导致加载失败。PLUGIN_PTY_SNAPSHOT_BACKEND 同样拒绝未知键。两个插件在加载时都不会 spawn PTY。

## 模型体验

间接通过 terminal 工具消费方。本注册表不贡献工具 schema 或提示词。

#### KV Cache 影响

无直接失效。

## 已知限制与延后工作

- 本 crate 只提供内存 snapshot 后端；真实 bash PTY 分配在 `dsh-terminal-bash`。
- 会话是进程内的，harness 重启后不会恢复。
- 本阶段不把这些插件挂入 `base.cordis.yml`。
