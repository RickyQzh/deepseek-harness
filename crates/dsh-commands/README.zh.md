# dsh-commands

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的进程内 slash 命令注册表。本 crate 不提供 HTTP，也不注册产品命令。

`CommandRegistry::parse` 是关联函数。一行在 `/` 位于字节 0、名称为 `[a-z][a-z0-9_-]*`、且其后为字符串结尾或 ASCII 空格/制表符/CR/LF 时匹配。`/foo bar` 得到名称 `foo`，`raw_input` 为 ` bar`（分隔符留在 `raw_input` 中）。以数字开头的名称（`/1foo`）、大写名称、以及不在索引 0 的斜杠得到 `None`。

`register` 插入一条定义并返回 `Dispose`。重复名称是 `RegisterError::Duplicate`。`dispose(self)` 与 `Drop` 只注销一次。`list` 返回按名称排序、不含 handler 的描述符。空注册表合法。

`execute` 先解析再按名称查找。解析失败或未知名称返回 `None`，且不写日志。已注册名称运行 handler，返回 `Some(CommandExecution)`，`command_id` 为 `cmd-{pid}-{nanos}`。Handler 类型为 `Arc<dyn Fn(ParsedCommand) -> BoxFuture<'static, CommandResult> + Send + Sync>`。本 crate 不追加会话事件。

`plugin::register` 挂载 YAML `@deepseek-ai/dsh-commands` 并提供 `commands`。配置为 `{}` 或省略。未知键会使加载失败。该插件不注册任何命令定义。

## 已知限制与暂缓事项

- Slash RPC 与宿主插件装配不在本 crate；`dsh-base` 与 `dsh-cli` 不注册此插件。
- 未实现会话的 `command/run` 与 `command/done` 事件、按 agent（智能体）分层的作用域，以及产品命令（`/compact`、`/plan`）。
