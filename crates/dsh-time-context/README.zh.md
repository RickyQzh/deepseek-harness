# dsh-time-context

[English](README.md) | 中文

面向 Rust 宿主的可选 pre-step 时钟注入。`plugin::register` 读取可选 YAML `timeZone` 与 `refreshIntervalMs`。未知键以及负数或非整数的 `refreshIntervalMs` 会在加载时失败。YAML 名称为 `@deepseek-ai/dsh-time-context`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

`agent/pre-step` 监听器在非空的 `Enter { messages }` 上前置一条用户角色的 `MessageSource::Plugin { plugin: "time-context" }` 消息，然后调用 `next(decision)`。Reject 与空 Enter 保持不变（不会制造虚假 step）。省略 `refreshIntervalMs` 或设为 `0` 时在每个合格 step 注入；正值仅在会话尚无更早的 time-context 注入、或距该事件 `time` 的已过毫秒数至少达到该间隔时注入。非法的 `refreshIntervalMs` 会失败，且错误信息包含 `refreshIntervalMs`。

每次读数两行：`Time sampled while preparing turn {turn}, step {step}: {YYYY-MM-DD HH:MM:SS UTC}` 以及 `Elapsed since the preceding {baseline}: {elapsed}.`。step 1 的 `baseline` 为 `model-visible message`，之后为 `step context`。时长使用紧凑整秒单位（`{days}d {hours}h {minutes}m {seconds}s`，始终包含秒；零时长为 `0s`）；缺少基线时为 `unavailable`。墙钟时间由 `std::time::SystemTime` 格式化为 UTC。

## 已知限制与暂缓事项

- 本阶段不做浏览器时区推导，也不按 IANA `timeZone` 显示；时间戳仅为标准 UTC（`YYYY-MM-DD HH:MM:SS UTC`）。
- 本阶段不把该插件挂入 `base.cordis.yml`。
