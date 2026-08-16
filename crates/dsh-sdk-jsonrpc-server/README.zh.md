# dsh-sdk-jsonrpc-server

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 NDJSON JSON-RPC SDK 服务器与 `dsh-jsonrpc-agent` stdio 二进制。

`register` 挂载 YAML 名称 `sdk-jsonrpc-server`。该插件注入 `agents` 与 `sessions`，绑定 stdin/stdout，提供 `sdkJsonRpcServer`，并且从不把诊断写到 stdout：stdout 只承载帧；加载与持久化失败向 stderr 打印 `dsh: {message}` 并以退出码 1 退出。bin 在 `boot_yaml` 之后才开始 serve，因此 `initialize` 能看到兄弟插件已注册的适配器。握手 `initialize` `{cwd, provider, model, maxTokens?}` 应答 `{serverInfo: {name: "deepseek-harness-sdk-runtime", version: "0.0.1"}}`。若存在 `maxTokens`，它必须是正整数。不支持再次 initialize。`session/prompt` `{sessionId, contentBlocks}` 立即返回 `{messageId}`；第一个未知 id 会创建 agent（智能体）。通知为 `session.event`（完整信封）、`session.status`（`idle` 或 `running`）、`subagent.started` 与 `subagent.finished`，经单一 FIFO 任务按入队顺序写出，因此 `session.status` idle 不会抢在 inbox splice 或 assistant 文本之前。`bind` 在共享的 `Context` 上监听内核事件 `subagent/start` 与 `subagent/end`，并把 `Completed` 映射为 SDK 状态 `ok`（其他停止原因映射为 `error`）。`shutdown` 应答 `{}`。bin 调用 `dsh_base::register_base_plugins`，因此同进程 subagent YAML 名称存在。当 `$DSH_CORDIS_CONFIG` 已设置且非空时使用该路径的配置 YAML，否则使用附带的 `minimal.cordis.yml`，其 mock 适配器占用 `deepseek-official`，因此 initialize 不需要 API 密钥。`BASE_YAML` 是附带的第 6 阶段 `base.cordis.yml`。缺失的 YAML `name` 仍会通过 boot 大声失败。JSONL 持久化在 `DSH_SESSION_ROOT` 已设置且非空时使用该目录，否则使用 `{DSH_HOME}/sessions`；当两者都未设置时，二进制将 `DSH_HOME` 设为 `$HOME/.dsh`。

## 已知限制与暂缓事项

- 未实现持久 shell。
- 未实现 `str_replace_editor`。
