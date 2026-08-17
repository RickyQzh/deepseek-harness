# dsh-tool-terminal

[English](README.md) | 中文

六个面向模型的 `terminal_*` 工具，构建于 [`dsh-terminal`](../dsh-terminal/README.md) 之上：`terminal_open`、`terminal_send`、`terminal_read`、`terminal_signal`、`terminal_close` 和 `terminal_list`。YAML 名称为 `@deepseek-ai/dsh-tool-terminal`。Owner 身份是 `ToolExecution.session_id`（`SessionId`）；缺少 session 时以 `terminal tools require an initiating session` 失败关闭。本 crate 不依赖 `dsh-agent`、`dsh-acp` 或 `dsh-jobs`。它不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

本 crate 中的 `terminal_send` 只走前台。`enableRunInBackground` 默认 **true** 会在 schema 中公布 `run_in_background`，并在描述后追加 ` Background mode returns a job id for job_output/job_kill.`；`run_in_background: true` 在挂载 jobs 之前仍返回 `background terminal sends require @deepseek-ai/dsh-jobs and @deepseek-ai/dsh-tool-jobs`。当 `enableRunInBackground` 为 false 时，schema 省略 `run_in_background`，强制传入 `true` 会以 `background terminal sends are disabled by tool-terminal configuration` 失败。`submit` 默认为 true。关闭原因是 `model request`。对正在关闭的会话并发 `kill` 会渲染 `terminal session {id} was already closing`。

`maxResultBytes` 默认为 `256 * 1024`，最小为 `64`。未知配置键会使加载失败。空配置使用默认值。截断在 `render.rs` 内按 UTF-8 字节进行（`\n[output truncated]`）。呈现辅助函数（`present_*`）是参数的纯函数，不存放在 `ToolDefinition` 上。

## 配置

| 键 | 默认 | 含义 |
|---|---|---|
| `enableRunInBackground` | `true` | 公布并接受 `run_in_background`。为 false 时省略该 schema 字段。 |
| `maxResultBytes` | `262144` | 一次完整 terminal 结果的 UTF-8 上限（最小 `64`）。 |

```yaml
- name: '@deepseek-ai/dsh-tool-terminal'
  config:
    enableRunInBackground: true
    maxResultBytes: 262144
```

## 模型体验

模型看到六个 `terminal_*` 工具以及如下 `tool:pty` 段落（顺序 106）：

```markdown
Use a terminal session only when work needs persistent terminal state or interactive stdin; prefer shell/read/write/edit for bounded one-shot operations. Track every terminal session id and close sessions that no longer matter. An inferred_idle or timeout result does not prove the foreground command exited.
```

#### KV Cache 影响

指引在前缀上保持稳定；工具结果在可复用请求前缀之后只追加。

## 已知限制与延后工作

- 在挂载 `dsh-jobs` 与 `dsh-tool-jobs` 之前，后台 `run_in_background` 发送会返回需要 jobs 的那句错误。
- `register` 不会从 `register_base_plugins` 调用。
- 尚未组装具名 ACP `pty-tools`。
