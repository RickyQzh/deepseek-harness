# dsh-acp

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 ACP（Agent Client Protocol）stdio 适配器。本 crate 不是 SDK JSON-RPC 服务器（`dsh-sdk-jsonrpc-server`）。帧格式为 NDJSON JSON-RPC 2.0，不是 LSP Content-Length。stdout 只承载这些 ACP 帧。

`register` 挂载 YAML `@deepseek-ai/dsh-acp`（`dsh_boot::PLUGIN_ACP`）。`register_acp_plugins` 调用 `register`。插件 setup 注入 `agents` 与 `sessions`，绑定 stdin/stdout，安装权限监听器，提供 `acpServer`，并且不 serve；`ACP_YAML` 是捆绑的组合（mock 文本 `acp-ok`，审批 `ask`，无自动批准）。配置要求字符串 `provider` 与 `model`。本 crate 不依赖 `dsh-sdk-protocol`、`dsh-sdk-jsonrpc-server`、`dsh-base`、`dsh-host`、`dsh-cli`、`dsh-headless` 或 `dsh-subagent`。`dsh-agent` 不得依赖本 crate。

`initialize` 忽略客户端的 `protocolVersion`，始终返回 `protocolVersion` 1、`agentInfo.name` 为 `deepseek-harness-acp`、`agentInfo.version` 为 `0.0.1`、空的 `authMethods`，以及 `image`、`audio` 与 `embeddedContext` 均为 false 的 `promptCapabilities`；不序列化 `sessionCapabilities` 与 `mcpCapabilities`。`authenticate` 返回 `{}`。`session/new` 要求绝对路径 `cwd`；接受空的 `additionalDirectories` 与 `mcpServers`；非空列表返回 JSON-RPC `-32602`。未知请求（包括 `session/load`）返回 JSON-RPC `-32601`，消息为 `"Method not found": {method}`，并带有 `data.method`。

`acp_prompt_to_text` 按原样拼接 `text` 块，并把每个 `resource_link` 渲染为带方括号的 `[resource_link name=<json-string> uri=<json-string>]` 引用；其他块不贡献文本。`prompt_has_unsupported_content` 在任一块既不是 `text` 也不是 `resource_link` 时为 true。`turn_end_to_stop_reason` 把 harness 的 `TurnEndReason` 映射为 ACP `StopReason`（`MaxTokens` → `max_tokens`）。`prompt_stop_reason` 是 prompt RPC 映射：缺失结束原因 → `cancelled`，`MaxTokens` → `end_turn`，其余走 `turn_end_to_stop_reason`。

`session/prompt` 等待整个 agent 空闲，发出已提交的 `agent_message_chunk` 文本，并报告 `end_turn`（包含 token 上限结束）。`session/cancel` 对未知 id 为空操作，并把进行中的提示词结算为 `cancelled`；同一会话已有进行中的提示词时，第二个 `session/prompt` 返回 `-32602`。一条连接拥有多个会话，各有独立的提示词 slot。stdio EOF 与 `quiesce` 会取消进行中的提示词并 unregister 这些会话；不 drain 可继续后代。

桥接器对所拥有的会话应答 `approval/request`，发送一次性的 `session/request_permission`（`allow-once` / `reject-once`）；未知 option id 映射为 `rejected`，客户端错误映射为 `unavailable`，并且不会授予持久权限。

## 已知限制与暂缓事项

- 不支持会话 load、list、resume、delete 与 fork。
- 不支持 MCP、PTY 与 LSP。
- 不支持可继续后代 drain。
- 具名快照子集仅为 handshake、reject-extra-dirs 与 text-turn。
