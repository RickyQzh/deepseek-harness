# dsh-subagent-in-process

[English](README.md) | 中文

同进程一次性 spawn 与 fork 提供方，以及共享的子 agent 驱动器。两个 YAML 名称：`@deepseek-ai/dsh-subagent-spawn-in-process`（默认提供方 `spawn`）与 `@deepseek-ai/dsh-subagent-fork-in-process`（默认提供方 `fork`）。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

Spawn 设置 `inherits_parent_context = false`，并启动全新的子会话。Fork 设置 `inherits_parent_context = true`，并用父级事件直到最后一个 `turn/end`（含）为子会话播种（没有已完成轮次时种子为空）。两者都将每项启动时能力声明为 true。一次性 `start` 从不调用 `prepare_continuable`。

`start_in_process_run` 生成 `SessionId` `sub-{pid}-{nanos}`，写入 `parent_session`、`origin = subagent`、`delegation_depth = parent + 1`，在父级有 `cwd` 时复制它，追加第 2 版 descriptor `{ mode: "one-shot", provider, label }`，经 `AgentRegistry::resume` 恢复（不用 `create`，因为 `create` 会把 `parent_session` 强制为 `None`），从 `parent.lock()` 同步复制父级 provider/model/max_tokens 且不在 `.await` 上持有该守卫，在**子**句柄上 `followup` 提示词，然后只对子句柄调用 `run_until_idle`。它从不占用父级 driver 许可。在发出 `subagent/start` 之后，子轮次完成时以及 `followup` 或 `run_until_idle` 失败时（停止原因 `Error`）都会发出 `subagent/end`。`TurnEndReason::Blocked` 映射为 `SubagentStopReason::Refusal`。结果 `output` 是 `seed_length` 之后子会话自己后缀中的 assistant 文本。

同一 kernel context 上需要 `agents`。缺少 `agents` 会在 start 时失败。

## 配置

| 键 | 默认 | 含义 |
|---|---|---|
| `providerName` | `spawn` 或 `fork` | `subagents` 上的注册表名称。 |

未知键会在加载时失败。空的 `providerName` 会在加载时失败。

## 模型体验

### 子 agent 请求

#### 模型看见什么

Spawn 把任务作为子会话唯一的用户消息送达。Fork 先附上父级已完成轮次的消息，再附上任务。

#### Token 影响

Spawn 为全新独立上下文付费。Fork 把已完成的父级前缀复制进子会话。

#### KV Cache 影响

与父级请求缓存相互独立。

## 已知限制与延后工作

- 本路径不会启动可继续的 fork/spawn（`prepare_continuable` 的消费方）。
- 本阶段声明了 `outputSchema`、`toolFilter` 与 `persona`，但未应用它们。
- 本阶段不把这些插件挂入 `base.cordis.yml`。
