# dsh-tool-jobs

[English](README.md) | 中文

面向模型的 `job_output`、`job_list` 与 `job_kill`，构建于 [`dsh-jobs-local`](../dsh-jobs-local/README.md) 之上。YAML 名称为 `@deepseek-ai/dsh-tool-jobs`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。若未提供 `agents`，则跳过完成通知，插件仍会加载。

参数名是 `id`，不是 `job_id`。`job_output` 为 `{ id, wait?, timeout_ms? }`。`job_list` 为 `{}`。`job_kill` 为 `{ id }`。空 `id` 校验失败。未知 id 是包含 `unknown job` 的工具错误。读取结果渲染正文或 `(no new output)`，然后是 `[status: …]`。取消仍在运行的工作返回 `requested cancellation of job {id}`。取消已经终止的任务返回 already-finished 文本。`wait: true` 超时返回当前快照，不是工具错误。

未上报的、有 owner 的完成使用 `MessageSource::Plugin { plugin: "tool-jobs", form: Some("notice"), compaction_id: None, source_command_id: None }`。默认 `wakeup` 投递下，空闲 owner 通过 `AgentHandle::followup` 被唤醒；忙碌 owner 走 `inject`。连续唤醒达到 `maxConsecutiveWakes`（默认 3）之后，通知降级为 `inject`。`AgentHandle::lock()` 只同步使用。用户撰写的 pre-step 输入会重置唤醒预算。

## 配置

| 键 | 默认 | 含义 |
|---|---|---|
| `waitTimeoutMs` | `30000` | `wait: true` 省略 `timeout_ms` 时使用的等待时长。 |
| `maxWaitTimeoutMs` | `600000` | 模型提供的等待上限。 |
| `completionDelivery` | `wakeup` | `wakeup` 会为空闲 owner 开启一轮；`quiet` 则 inject。 |
| `maxConsecutiveWakes` | `3` | 一个 owner 可通过唤醒开启的轮次数，超过后通知降级为 inject。 |

未知键会在加载时失败。默认等待超过上限会在加载时失败。

## 模型体验

模型看到 `job_output`、`job_list` 与 `job_kill`。结果以 `[status: …]` 结尾。未上报的、有 owner 的完成以插件通知到达。

#### KV Cache 影响

可复用请求前缀之后，工具结果与通知只追加。

## 已知限制与延后工作

- `ToolExecution` 没有调用方 session；工具读取 `CompactionScope`（工具 body 期间未设置），因此除非后续阶段传入调用方，否则只能看见无 owner 的任务。
- 在 `SystemPrompt` 支持提供之后再登记段落之前，省略 `tool:jobs` 系统提示词段落。
- 未实现 isolate / preset 控制器分层。
- 本阶段不把该插件挂入 `base.cordis.yml`。
