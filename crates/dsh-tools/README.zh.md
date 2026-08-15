# dsh-tools

[English](README.md) | 中文

本 crate 为 Rust 宿主提供工具执行类型，以及无损 JSON 参数冻结。流水线（pre-execute、approval、guards、execute、post-execute、finalize）与 Code Mode collapse 位于本 crate；bash 与文件系统工具不在此。

`freeze_args` 克隆一份 `serde_json::Value`，使策略监听器拿到独立副本。`freeze_args_from_raw` 将空的模型字符串映射为 `{}`，并将非法 JSON 保留为字符串，与 TypeScript 循环中的 `parseArguments` 一致。产品 call id 使用 `dsh_session::CallId`。

## 已知限制与暂缓事项

- 作用域限制、`presentAs` 以及 `run_code` worker 属于后续阶段。本 crate 只实现 collapse-before-policy。
