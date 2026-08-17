# Agent Note: Phase 7 GUI RpcMethodMap and slash remotes in dsh-host

Status: implemented

[English](2026-08-17-rust-gui-rpc-method-map.md) | 中文

## 问题

第 7 阶段的 Rust GUI 宿主必须回答其余点分 `RpcMethodMap` 方法，以及 TypeScript SPA 已经调用的两条 Typert slash Remote，且不组合 `web.cordis.yml`、不依赖 `dsh-subagent`。若这些名字仍是载体 HTTP 404，loopback 上的 workspace create、settings describe、skill list 与 slash 命令发现会空白。

## 决策

`dsh-host` 在 `SessionHandler` 旁挂载 `GuiHandler`。`spawn_stub_host` 仍是 `StubHandler`。组合映射的 HTTP 测试通过 `spawn_gui_host` / `listen_with_handler` 监听。

`/api` 信任围栏之后的分发是 JSON Content-Type 与 `client-request` 解析，然后 `method` 等于 `/api/` 之后的路径后缀，再对点分名字做特权再检查（`is_privileged_method` / `privileged_requires_loopback`）。若该后缀含 `/`，走 slash 拦截器；否则走 `accepts_dotted`。专用的 `POST /api/respond` 以及 `GET`/`HEAD` `/api/events.mux` 与 `/api/events.host` 从不进入拦截器。

slash Remote 只有 payload 为 `{ args }` 的 `commands/list` 与 `commands/execute`。list 返回 `{ commands: [{ name, description }] }`（空数组也是成功）。execute 先 `CommandRegistry::parse` 再 `execute`；未命中是 HTTP 200 的 `RpcResult::err` `unknown-command`。其它 `/api/foo/bar`（含 `goals/create`）为 HTTP 404。

点分 `GuiHandler` 复用 `WorkspaceRegistry`、`SettingsService`、`LayeredCredentials`、`CommandRegistry`、`SkillRegistry::list` 与 `SessionHandler`。`host.describe` 用 `AgentRegistry::list().len()` 填 `attachedSessions`，可选的 `provider`/`model` 取自第一个 LLM（大语言模型） provider 或 lookup 默认值。`host.listDirectory` 只列按名排序的目录，上限 500 后 `truncated: true`。`host.pickDirectory` 为 `directory-picker-unavailable`；`host.openPath` 为 `internal`，且 `canOpenPath` 保持 false。点分 `goal.*` 为 `internal` `"goals are not implemented in Phase 7"`。`agentPreset.list` 是只读的 `standard` 名册；编写方法为 `agent-preset-read-only`。`subagent.list` 返回 `{ items: [] }`，不发明 ACP（Agent Client Protocol）。`dsh-agent` 不依赖 `dsh-host` 或 `dsh-subagent`；`dsh-host` 不依赖 `dsh-subagent`。

线路冻结（含 slash 命名空间 404 与特权集合）仍在[冻结 Rust GUI 宿主的四象限线路](../../proposed/architecture/2026-08-16-rust-gui-host-wire.md)。

## 测试

`workspace_create_via_http`、`skill_list_returns_name_and_description`、`agent_preset_list_is_standard_readonly`、`settings_describe_loopback_exposes_ui_onboarding`、`slash_commands_list_empty_array`、`slash_goals_create_is_http_404` 与 `privileged_settings_describe_trusted_non_loopback_is_403` 钉住该映射。`cargo test -p dsh-host --offline` 保持一元、WebSocket 与 session 测试。

## 考虑过的替代方案

**让 `spawn_stub_host` 指向 `GuiHandler`。** 不予采用：载体测试必须保持 `StubHandler`，使该监听器上唯一已安装的点分名字仍是 `host.describe`。

**移植每一个 Typert slash Remote。** 线路冻结已否决：第 7 阶段只安装 `commands/list` 与 `commands/execute`；未知 slash 命名空间保持 HTTP 404。

**把 `dsh-subagent` 加进 `dsh-host`，或把 `dsh-host` 加进 `dsh-agent`。** 不予采用：`subagent.list` 返回空目录；此处不发明 ACP。

**把 slash `goals/create` stub 成点分 `goal.create` 成功。** 不予采用：slash `goals/create` 未安装（HTTP 404）；点分 `goal.*` 回答 `internal`。

## 后果

未安装的 slash 命名空间会让 TypeScript UI 降级，而不是与假宿主双跑。来自受信任非 loopback Host 的特权 `settings.describe` 仍是 HTTP 403。捆绑的 `web.cordis.yml` 图由[web 插件图笔记](2026-08-17-rust-web-plugin-graph.md)持有；`dsh-cli web` 仍是后续第 7 阶段任务。
