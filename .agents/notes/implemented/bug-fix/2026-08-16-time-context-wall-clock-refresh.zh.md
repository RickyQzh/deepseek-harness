# Agent Note: Time-context 刷新使用进程内墙钟

Status: implemented

[English](2026-08-16-time-context-wall-clock-refresh.md) | 中文

## 问题

`crates/dsh-time-context` 把 `unix_now_ms()`（纪元毫秒）与 `SessionEvent.time` 比较。LoopAgent 把 `time` 写成 `seq as i64`，不是墙钟。因此正值 `refreshIntervalMs` 从不跳过（`now - seq` 约为 1.7e12 毫秒），时长文本会格式化成数百万天，而不是 `unavailable`。

TypeScript 插件可以从持久事件时间戳调度，因为那些值是纪元毫秒（[持久的逐步骤时间上下文](../feature/2026-07-16-durable-per-step-time-context.md)）。Rust 宿主不能。

## 决策

`dsh-time-context` 不把日志 `time` 当作墙钟。插件本地的 `HashMap` 以会话 id 字符串为键，记录上次前置的墙钟毫秒，与 token-meter 的会话 id 键方式相同。仅在监听器实际前置时更新该映射。省略 `refreshIntervalMs` 或设为 `0` 时仍在每个合格 step 注入。

当前置事件缺失、或其 `time` 小于 `1_000_000_000_000`（seq 充当 time 以及其他非 Unix 毫秒值）时，时长文本为 `unavailable`。可信的 Unix 毫秒基线仍使用 `format_duration`。LoopAgent 的 seq 充当 time 约定不变。

## 考虑过的替代方案

**把 `unix_now_ms()` 与 `event.time` 比较。** 这就是混用时钟的缺陷：seq 不是纪元毫秒，所以线上正值间隔从不跳过，时长文本会变成数百万天。

**把 LoopAgent 的 `time` 改成纪元毫秒。** 这会恢复 TypeScript 的持久间隔约定，但 seq 充当 time 是宿主范围的约定（compaction、会话追加、repair）。本 crate 不改它。

**加入 chrono 或其他时间 crate。** `SystemTime` 加手工 UTC 格式化已经提供 `now_ms` 和时间戳行；再加一个日历 crate 也不会改变 seq 充当 time 的日志。

## 后果

在同一进程、同一插件实例内，正值间隔会在相同的模拟 `now_ms` 下跳过第二次合格 step，并在墙钟前进达到间隔后再注入。resume、重启或新的插件实例看到空映射，因而会注入。典型 harness 日志上的时长为 `unavailable`。包测试钉住这两种行为；`format_duration` 用例不变。
