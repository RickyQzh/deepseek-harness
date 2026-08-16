# dsh-llm

[English](README.md) | 中文

面向 Rust 宿主的提供方无关 LLM（大语言模型）流契约。适配器在终止 `finish` 之前发出 `usage`，之后不再发出任何分片。`LlmRuntime::stream` 将适配器失败转换为该终止 `finish`（`error` 或 `aborted`）；消费方不捕获抛出的适配器错误。`LlmRuntime` 可 `Clone`（适配器映射），`list_providers` 返回已注册的路由 id。

`StreamChunk`、`FinishReason`、`TokenUsage` 和 `Message` 是第 2 阶段的 `dsh-session` 类型。`ToolSchema` 在此声明，因为它随 `GenerateOptions` 传递。`BlockAssembler` 是循环把分片组装成消息的步骤，包括在 max-tokens 时丢弃 tool-call 块的规则。

`plugin::register_llm` 提供 `llm`。`plugin::register_mock` 按配置 `provider`（默认 `mock`）注册 `MockAdapter`。`replay::register` 从 `DSH_SNAPSHOT_FILE` 按配置的每个 `providers[].id` 提供 `assistant/chunk` 轮次。

## 已知限制与暂缓事项

- 适配器注册表的替换/dispose（资源释放）、可配置提供方目录、模型发现以及 `dsh-llm-retry` 属于后续阶段。第 3 阶段是适配器的 `HashMap` 加上 `prepare_call` 填入适配器默认值。
