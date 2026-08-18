# dsh-agent-loop

[English](README.md) | 中文

面向 Rust 宿主的脚本化 agent loop（智能体循环）驱动器：持久化 inbox 拼接、idle / maintenance / running 相位机、运行时上下文快照同一性、从 `derive_messages` 与 `request/header` 重建请求，以及工具调用调度。

`followup` / `steer` / `inject` / `cancel` 只改动 inbox 和 abort 标志。`run_until_idle` 是轮次驱动器。一轮次总是在首次 claim 之前追加 `turn/start`；首次 enter 为空时仍记录 `turn/end` completed，且不开启 step。`LoopAgent::new` 接收 kernel 的 `Context` 以及共享的 `Arc<Mutex<ToolRuntime>>` / `Arc<Mutex<LlmRuntime>>`。pre-step 准入是作用于 `PreStepDecision` 的 `agent/pre-step` waterfall（`EVENT_AGENT_PRE_STEP`）；监听器不调用 `next()` 即短路。模型请求恢复是作用于 `RequestErrorAction` 的 `agent/request-error` waterfall（`EVENT_AGENT_REQUEST_ERROR`）；默认 `Fail`，`Retry` 在同一步骤内重复请求。`LoopAgent` 在这些 waterfall 外包一层任务局部的 `CompactionScope`（Exclusive 与 Shared）以及 `dsh_llm::retry::RetryScope`，使压缩（compaction）能在不跨越摘要器 `.await` 持有 `Mutex<LoopAgent>` 的情况下改写实时会话，并在结束后追加排空的 `llm/retry` / `llm/retry-started` 记录（`seq`/`time` = 当前日志长度）。`run_until_idle_locked` 驱动同一循环，仅在同步会话变更期间锁定 `Mutex<LoopAgent>`。

`max-tokens` 步骤在该轮次内保持：后续 completed 步骤不会覆盖它，下一轮次开始时没有残留原因。空闲时的 `cancel` 是空操作。工具调度中止时会排空已启动的调用，并为尚未启动的调用记录 `ABORTED_BEFORE_DISPATCH`。中止之后 `turn` 返回 false，因此当前的 `run_until_idle` 不会再开一轮；inbox 中剩余条目等待下一次驱动。独占工具一次只运行一个；可并行工具共享 `max_parallel_tool_calls`。可并行调用串行运行 `ToolRuntime::prepare`，重叠 `dispatch` body，然后 `finalize`。`prepare` 和 `finalize` 在 Tokio 阻塞线程池上持有 tools 的 `Mutex`，以便运行时 worker 仍可轮询进行中的 dispatch future 以及 `cancel`。

## 已知限制与暂缓事项

- 工厂 `create` / `resume` 以及 kernel 插件包装属于第 5 阶段。实时注册表位于 `dsh-agent`。本 crate 直接持有 `LoopAgent`。Bash / fs 工具属于第 4 阶段；测试注册 mock 工具。
