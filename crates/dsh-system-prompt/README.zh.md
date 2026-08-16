# dsh-system-prompt

[English](README.md) | 中文

为 Rust 宿主组装有序的系统提示词段落、动态运行时上下文快照、工具 schema 和提示词变量。`{{var}}` 插值是严格的：未知、未定义和格式错误的引用会失败；单独出现且后面没有 `}}` 的 `{{` 视为字面散文；替换后的值不会再次扫描。

Harness 身份是顺序为 −100 的 `harness:identity` 段落。部署 persona 是顺序为 0 的 `deployment:persona`。`TOOL_ORDER_REST`（`<unlisted-tools>`）标记未列出的工具按字典序插入的位置。快照同一性（渲染文本未变化时跳过新的用户消息）由 `dsh-agent-loop` 应用，而不是本 crate。

`plugin::register` 根据 YAML 的 `persona`、`includeHarnessIdentity`（默认 true）和 `includeRuntimeContext`（默认 false）提供 `systemPrompt`。

## 已知限制与暂缓事项

- 作用域段落/变量遮蔽以及 Cordis `system-prompt/assemble` waterfall 插件属于第 5 阶段。本 crate 是带进程内监听器列表的库。
