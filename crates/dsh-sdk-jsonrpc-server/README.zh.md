# dsh-sdk-jsonrpc-server

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的 NDJSON JSON-RPC SDK 服务器与 `dsh-jsonrpc-agent` stdio 二进制。

`register` 挂载 YAML 名称 `sdk-jsonrpc-server`。该插件注入 `agents` 与 `sessions`，绑定 stdin/stdout，提供 `sdkJsonRpcServer`，并且从不把诊断写到 stdout：stdout 只承载帧；加载与持久化失败向 stderr 打印 `dsh: {message}` 并以退出码 1 退出。bin 在 `boot_yaml` 之后才开始 serve，因此 `initialize` 能看到兄弟插件已注册的适配器。握手 `initialize` `{cwd, provider, model, maxTokens?}` 应答 `{serverInfo: {name: "deepseek-harness-sdk-runtime", version: "0.0.1"}}`。若存在 `maxTokens`，它必须是正整数。不支持再次 initialize。`session/prompt` `{sessionId, contentBlocks}` 立即返回 `{messageId}`；第一个未知 id 会创建 agent（智能体）。通知为 `session.event`（完整信封）与 `session.status`（`idle` 或 `running`），经单一 FIFO 任务按入队顺序写出，因此 `session.status` idle 不会抢在 inbox splice 或 assistant 文本之前。`subagent.started` 与 `subagent.finished` 存在于 `dsh-sdk-protocol`，本服务器从不发出它们。`shutdown` 应答 `{}`。当 `$DSH_CORDIS_CONFIG` 已设置且非空时使用该路径的配置 YAML，否则使用附带的 `minimal.cordis.yml`，其 mock 适配器占用 `deepseek-official`，因此 initialize 不需要 API 密钥。缺失的 YAML `name` 仍会通过 boot 大声失败。JSONL 持久化在 `DSH_SESSION_ROOT` 已设置且非空时使用该目录，否则使用 `{DSH_HOME}/sessions`；当两者都未设置时，二进制将 `DSH_HOME` 设为 `$HOME/.dsh`。

## 已知限制与暂缓事项

- 本 crate 不挂载同进程 subagent。
- 未实现持久 shell。
- 未实现 `str_replace_editor`。
