# dsh-sdk-jsonrpc-server

English | [中文](README.zh.md)

NDJSON JSON-RPC SDK server and `dsh-jsonrpc-agent` stdio binary for the DeepSeek Harness Rust host.

`register` mounts YAML name `sdk-jsonrpc-server`. The plugin injects `agents` and `sessions`, binds stdin/stdout, provides `sdkJsonRpcServer`, and never writes diagnostics to stdout: stdout carries frames only; load and persist failures print `dsh: {message}` to stderr and exit 1. The bin serves after `boot_yaml` so `initialize` sees sibling adapter registration. Handshake `initialize` `{cwd, provider, model, maxTokens?}` answers `{serverInfo: {name: "deepseek-harness-sdk-runtime", version: "0.0.1"}}`. `maxTokens`, when present, must be a positive integer. Re-initialize is unsupported. `session/prompt` `{sessionId, contentBlocks}` returns `{messageId}` immediately; the first unknown id creates the agent. Notifications are `session.event` (full envelope), `session.status` (`idle` or `running`), `subagent.started`, and `subagent.finished`, written in enqueue order through one FIFO task so `session.status` idle cannot overtake the inbox splice or assistant text. `bind` listens for kernel `subagent/start` and `subagent/end` on the shared `Context` and maps `Completed` to SDK status `ok` (any other stop reason to `error`). `shutdown` answers `{}`. Config YAML is `$DSH_CORDIS_CONFIG` when that variable is set and non-empty, otherwise the bundled `minimal.cordis.yml`, whose mock adapter claims `deepseek-official` so initialize needs no API key. Missing YAML `name` still fails loud via boot. JSONL persist uses `DSH_SESSION_ROOT` if set and non-empty, else `{DSH_HOME}/sessions`; when neither is set the binary sets `DSH_HOME` to `$HOME/.dsh`.

## Known Limitations and Deferred Work

- This crate does not mount in-process subagents.
- Persistent shell is not implemented.
- `str_replace_editor` is not implemented.
