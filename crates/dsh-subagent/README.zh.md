# dsh-subagent

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的 subagent Service Definition：在 `subagents` 上的具名提供方注册表（`SubagentRuntime`），带能力检查的一次性 `start`，以及可继续的 `start_continuable`。YAML 名称为 `@deepseek-ai/dsh-subagent`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。同进程 spawn 与 fork 提供方在 [`dsh-subagent-in-process`](../dsh-subagent-in-process/README.md)。`dsh-agent` 不依赖本 crate。

`inject::<SubagentRuntime>()` 得到 `Arc<SubagentRuntime>`，因此 `register_provider`、`start`、`start_continuable` 与 `followup_child` 在内部 mutex 上取 `&self`。重复的提供方名称会失败。未知名称会失败。`start` 检查已声明的启动时能力，再委派给具名提供方；它不进入继续执行。`start_continuable` 发布子会话（`mode: "continuable"`），只把子 agent 驱动到 idle，并在返回前用 `followup` 向父级投递用户角色的 `MessageSource::SubagentSettled` 通知；它从不占用父级 driver 许可。`start_continuable_background` 在子会话发布后立即返回子 id。`followup_child` 向该子会话投递后续邮件，并在子 agent 空闲时只用 `run_until_idle` 驱动该子 agent。`list_continuable` 跟踪这些子 agent；一次性 fork 运行不会列入。`register_continuable_setup` 只向克隆出的子 `ToolRuntime` 贡献工具。`prepare_continuable` 是每个提供方上的数据桩，一次性 `start` 不会调用它。

`subagent/descriptor` 第 2 版是仅日志会话事件（`SessionEvent::SubagentDescriptor { data }`）。内核事件 `subagent/start` 与 `subagent/end` 不是会话事件。已发出 start 的已发布运行也必须发出 end，包括子轮次在发布后失败、以及向父级投递结算 `followup` 失败的情况。深度默认值为 3；当父级 `delegationDepth`（缺省为 0）加一超过上限时，`assert_subagent_max_depth` 会失败。

## 配置

无。未知键会在加载时失败。

## 模型体验

通过同进程提供方和 [`dsh-tool-subagent`](../dsh-tool-subagent/README.md) 间接影响。本注册表不贡献工具 schema。

#### KV Cache 影响

无直接失效。

## 已知限制与延后工作

- 未移植进程外提供方（ACP、Codex、Claude Code、SDK）。
- 本阶段不把该插件挂入 `base.cordis.yml`。
