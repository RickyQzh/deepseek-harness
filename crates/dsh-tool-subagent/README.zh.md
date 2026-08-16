# dsh-tool-subagent

[English](README.md) | 中文

面向模型的 `subagent`、`subagent_fork`、`send_message`、`list_agents` 与 `report`，构建于 [`dsh-subagent`](../dsh-subagent/README.md) 之上。四个 YAML 名称：`@deepseek-ai/dsh-tool-subagent`、`@deepseek-ai/dsh-tool-subagent-control`、`@deepseek-ai/dsh-tool-subagent-list` 与 `@deepseek-ai/dsh-tool-subagent-report`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

一行 `@deepseek-ai/dsh-tool-subagent` 把一个 `provider` 绑定到一个 `toolName`。`backgroundMode: continuable` 且 `run_in_background` 默认为 true 时调用 `start_continuable_background`，并立即返回 `{ kind: "continuable", subagentId }`。`backgroundMode: one-shot`（默认）等待 `SubagentRuntime::start`；`run_in_background: true` 在接入 jobs 之前是工具错误。Fork 保持一次性：配置 `{ provider: fork, toolName: subagent_fork, backgroundMode: one-shot }`。调用方身份是 `ToolExecution.session_id`。

`report` 只通过 `SubagentRuntime::register_continuable_setup` 注册到克隆出的子 `ToolRuntime`。它从不注册到父级共享运行时，因此一次性 fork 子 agent 看不见 `report`。`{ output }` 以用户角色的 `MessageSource::SubagentReport` 经 `followup`（`wakeup`）或 `inject`（`quiet`）投递，并返回 `{ messageId }`。`send_message` `{ subagent_id, message }` 用 `MessageSource::Coordinator` 调用 `followup_child`，并在子 agent 空闲时驱动它。`list_agents` 只列出可继续子 agent；一次性 fork 运行不会出现。`report` 的呈现为 generic。

## 配置

### `@deepseek-ai/dsh-tool-subagent`

| 键 | 默认 | 含义 |
|---|---|---|
| `provider` | 必填 | `subagents` 提供方名称。 |
| `toolName` | `subagent` | 面向模型的工具名。 |
| `enableRunInBackground` | `true` | 暴露 `run_in_background`；`false` 会省略该参数并拒绝强制后台调用。 |
| `backgroundMode` | `one-shot` | `one-shot` 等待 `start`；`continuable` 默认后台运行并返回子 id。 |
| `maxDepth` | `3` | 绝对深度上限，或 `"provider-managed"` 表示不发送上限。数值上限需要 `depthLimit`。 |

### `@deepseek-ai/dsh-tool-subagent-report`

| 键 | 默认 | 含义 |
|---|---|---|
| `reportDelivery` | `wakeup` | `wakeup` 使用 `followup`；`quiet` 使用 `inject`。 |

control 与 list 不接受任何键。未知键会在加载时失败。缺少 `provider` 会在加载时失败。

## 模型体验

模型看见 `subagent` / `subagent_fork`（按实例名称），以及在挂载对应插件时的 `send_message`、`list_agents` 与 `report`。可继续启动返回 `started subagent {id}`。一次性成功返回子 agent 的最终文本。结算以 `subagent-settled` 通知到达，与 `report` 无关。

#### KV Cache 影响

可复用请求前缀之后的追加式工具结果与通知。

## 已知限制与延后工作

- 在接入 jobs 服务之前，一次性 `run_in_background: true` 不可用。
- 未移植进程外提供方、`toolFilter`、`persona` 与 `agentOptions`。
- 本阶段不把这些插件挂入 `base.cordis.yml`。
