# dsh-base

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的第 6 阶段产品插件聚合器。`register_base_plugins` 注册审批（含 `headless-auto-approve`）、permission presets、llm-retry 以及 retry snapshot backend、token-meter、compaction-basic 以及 tool-result pruner、agent-instructions、time-context、skill（技能）三件套、web 加 DeepSeek search 加 tool-web、jobs-local 加 tool-jobs、subagent 加同进程 spawn/fork，以及四个 `dsh-tool-subagent` YAML 名称。

该函数不注册 spine、执行、headless 或 `sdk-jsonrpc-server` 插件。`dsh-agent` 不依赖 `dsh-subagent`；CLI（命令行界面）与 jsonrpc bin 调用 `register_base_plugins`，因此这些 YAML 名称存在。未设置 `DSH_CORDIS_CONFIG` 时，默认 YAML 仍是 `MINIMAL_YAML`。静态第 6 阶段组合树是 `dsh-headless/base.cordis.yml` 与 `dsh-sdk-jsonrpc-server/base.cordis.yml`（`BASE_YAML`）；这些文件省略 time-context、pruner、retry-snapshot-backend、workflow、goal、session-title、settings-file 与 commands，且不含 `!!js`。未知 YAML 名称仍然会大声失败。`register_base_plugins` 映射 `@deepseek-ai/dsh-mcp-client` 以及三个 PTY YAML 名称 `@deepseek-ai/dsh-terminal`、`@deepseek-ai/dsh-terminal-bash` 和 `@deepseek-ai/dsh-tool-terminal`，外加 `pty-snapshot-backend`；默认 YAML 不挂载 MCP 服务器或 PTY 行，且不得分配 PTY。

## 已知限制与暂缓事项

- 未设置 `DSH_CORDIS_CONFIG` 时仍启动 `MINIMAL_YAML`；当该变量指向该文件时才使用 `BASE_YAML`。
- `web_fetch` 保持关闭（`fetch: false`）。
- 未实现 `standard` profile。
