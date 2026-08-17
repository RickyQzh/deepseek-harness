# dsh-commands

English | [中文](README.zh.md)

In-process slash-command registry for the DeepSeek Harness Rust host. This crate does not serve HTTP and does not register product commands.

`CommandRegistry::parse` is an associated function. A line matches when `/` is at byte zero and the name is `[a-z][a-z0-9_-]*` followed by end-of-string or ASCII space, tab, CR, or LF. `/foo bar` yields name `foo` and `raw_input` ` bar` (the separator stays in `raw_input`). Digit-leading names (`/1foo`), uppercase names, and a slash not at index 0 yield `None`.

`register` inserts one definition and returns `Dispose`. A duplicate name is `RegisterError::Duplicate`. `dispose(self)` and `Drop` unregister once. `list` returns name-sorted handler-free descriptors. An empty registry is valid.

`execute` parses then looks up the name. A parse miss or unknown name returns `None` and logs nothing. A registered name runs the handler and returns `Some(CommandExecution)` with `command_id` `cmd-{pid}-{nanos}`. Handlers are `Arc<dyn Fn(ParsedCommand) -> BoxFuture<'static, CommandResult> + Send + Sync>`. This crate does not append session events.

`plugin::register` mounts YAML `@deepseek-ai/dsh-commands` and provides `commands`. Config is `{}` or omitted. Unknown keys fail load. The plugin does not register command definitions.

## Known Limitations and Deferred Work

- Slash RPC and host plugin wiring live outside this crate; `dsh-base` and `dsh-cli` do not register this plugin.
- Session `command/run` and `command/done` events, scoped per-agent layers, and product commands (`/compact`, `/plan`) are not implemented.
