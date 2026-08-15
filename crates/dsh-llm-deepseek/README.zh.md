# dsh-llm-deepseek

[English](README.md) | 中文

面向 Rust 宿主的 DeepSeek `POST /chat/completions` SSE（Server-Sent Events）适配器。该适配器序列化 harness 消息（assistant 的 `content` 为 `""`，绝不是 JSON `null`），解析 SSE 直至 `[DONE]`，翻译线路分片，并应用按次读取的空闲 watchdog。

每次 `stream` 调用都从该次调用的连接快照解析 bearer token。适配器结构体不存储密钥。HTTP 使用带 `rustls-tls` 的 `reqwest`（不用 OpenSSL / `native-tls`）。

## 已知限制与暂缓事项

- 模型目录、重试策略，以及状态映射之外的配额/上下文窗口分类器属于后续工作。第 3 阶段是针对进程内 mock 服务器的序列化 / SSE / 翻译 / 请求头 / 按次密钥 / 空闲超时。
