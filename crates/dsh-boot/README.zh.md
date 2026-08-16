# dsh-boot

[English](README.md) | 中文

DeepSeek Harness Rust 宿主的封闭 YAML 名称到 setup 闭包注册表与挂载。`boot_yaml` 解析 compose YAML、应用 `--patch` 文档、对每行的 `config` 插值，并把已注册插件挂载为 kernel fiber。未知的 YAML `name` 是加载错误，发生在该行 spawn 之前。compose 解析仍拒绝 `!!js`，且不会对其求值。

本 crate 不注册产品插件。调用方按 YAML `name` 在 `PluginRegistry` 上 `register` setup。spine 组合入口是 `dsh_agent::register_spine_plugins`。执行组合入口是 `dsh_agent::register_execution_plugins`。

## 已知限制与暂缓事项

- 产品插件 setup 不在本 crate 中；spine 的 YAML 名称由 `dsh_agent::register_spine_plugins` 注册，执行插件的 YAML 名称由 `dsh_agent::register_execution_plugins` 注册。
