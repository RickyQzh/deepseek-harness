# dsh-sandbox

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的失败即关闭的沙箱模式、可写根与提权。`SandboxMode` 使用 kebab-case（`read-only`、`workspace-write`、`danger-full-access`），且只命名文件效果。

`writable_roots` 在 `read-only` 与 `danger-full-access` 下为空；在 `workspace-write` 下是策略工作区根、`/tmp` 与平台临时目录去重后的规范路径集合。`canonical_path` 使用 `std::fs::canonicalize`，解析失败时返回输入的原样拼写。

`validate_escalation_args` 要求 `sandbox_permissions` 与非空 justification 成对出现。`approve_escalation` 按顺序失败即关闭：并非严格更宽、没有审批服务、没有 agent（智能体），然后才是审批结果。`SandboxPolicy::confined` 拒绝 `danger-full-access`。缺少后端时为 `SANDBOX_UNAVAILABLE`，并使用 TypeScript 拒绝段落（含 Windows ACL 子句）。

模型可见标记：`[sandbox: file access denied under {mode} mode]` 与 `[sandbox: escalation available — retry this exact {subject} once with sandbox_permissions (the narrowest wider mode that suffices) + justification; the approval prompt asks the user]`。

## 已知限制与暂缓事项

- 本 crate 不含 confine 包装、Landlock、bwrap 与 Seatbelt profile。
- resolve 不折叠 `sandbox/mode` 会话事件；解析出的模式即构造时的默认模式。
- 此处不选择 Darwin 与 Windows runner。
