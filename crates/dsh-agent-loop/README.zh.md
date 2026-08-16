# dsh-agent-loop

[English](README.md) | 中文

面向 Rust 宿主的脚本化 agent loop（智能体循环）驱动器：持久化 inbox 拼接、idle / maintenance / running 相位机、运行时上下文快照同一性、从 `derive_messages` 与 `request/header` 重建请求，以及工具调用调度。

`followup` / `steer` / `inject` / `cancel` 只改动 inbox 和 abort 标志。`run_until_idle` 是轮次驱动器。一轮次总是在首次 claim 之前追加 `turn/start`；首次 enter 为空时仍记录 `turn/end` completed，且不开启 step。

`max-tokens` 步骤在该轮次内保持：后续 completed 步骤不会覆盖它，下一轮次开始时没有残留原因。空闲时的 `cancel` 是空操作。工具调度中止时会排空已启动的调用，并为尚未启动的调用记录 `ABORTED_BEFORE_DISPATCH`。中止之后 `turn` 返回 false，因此当前的 `run_until_idle` 不会再开一轮；inbox 中剩余条目等待下一次驱动。独占工具一次只运行一个；可并行工具共享 `max_parallel_tool_calls`。可并行调用串行运行 `ToolRuntime::prepare`，重叠 `dispatch` body，然后 `finalize`。

## 已知限制与暂缓事项

- 工厂 `create` / `resume` 以及 kernel 插件包装属于第 5 阶段。实时注册表位于 `dsh-agent`。本 crate 直接持有 `LoopAgent`。Bash / fs 工具属于第 4 阶段；测试注册 mock 工具。
