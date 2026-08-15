# dsh-agent-loop

[English](README.md) | 中文

面向 Rust 宿主的脚本化 agent loop（智能体循环）驱动器：持久化 inbox 拼接、idle / maintenance / running 相位机、运行时上下文快照同一性，以及从 `derive_messages` 与 `request/header` 重建请求。

`followup` / `steer` / `inject` / `cancel` 只改动 inbox 和 abort 标志。`run_until_idle` 是轮次驱动器。一轮次总是在首次 claim 之前追加 `turn/start`；首次 enter 为空时仍记录 `turn/end` completed，且不开启 step。

## 已知限制与暂缓事项

- 工厂 `create` / `resume`、agent 注册表以及 kernel 插件包装属于第 5 阶段。本 crate 直接持有 `LoopAgent`。Bash / fs 工具属于第 4 阶段；测试注册 mock 工具。
