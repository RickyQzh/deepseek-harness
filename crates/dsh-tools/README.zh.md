# dsh-tools

[English](README.md) | 中文

本 crate 为 Rust 宿主提供工具执行类型、无损 JSON 参数冻结，以及 pre-execute / approval / guard / execute / post-execute 流水线。bash 与文件系统工具不在此。

`ToolRuntime::execute` 按该顺序运行。在 `ToolPresentationMode::Code` 下，模型直接调用已注册且不是 `run_code` 的名称会在 pre-execute 之前被拒绝（collapse-before-policy）。嵌套调用（设置了 `parent`）与未知名称不走 collapse：未知名称仍会运行 pre-execute，然后以 `unknown tool "{name}"` 失败。

`freeze_args` 克隆一份 `serde_json::Value`，使策略监听器拿到独立副本。`freeze_args_from_raw` 将空的模型字符串映射为 `{}`，并将非法 JSON 保留为字符串，与 TypeScript 循环中的 `parseArguments` 一致。产品 call id 使用 `dsh_session::CallId`。

## 已知限制与暂缓事项

- 作用域限制、`presentAs` 以及 `run_code` worker 属于后续阶段。本 crate 只实现 collapse-before-policy。
