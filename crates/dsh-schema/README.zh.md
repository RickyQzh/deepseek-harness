# dsh-schema

[English](README.md) | 中文

插件配置的 serde schema，以及给 Settings UI 用的 JSON Schema 文档。`Schema::validate` 检查 `serde_json::Value`。`Schema::to_json_schema` 返回带 `type` 字段、可供 UI 存储的 JSON 对象。

当插件名登记了 schema 时，`dsh-compose` 调用 `Schema::validate`。没有 schema 不是错误。

## 已知限制与暂缓事项

- 本 crate 不实现 Standard Schema / schemastery 适配器。那些内容留在 TypeScript 树中，直到宿主切换。
