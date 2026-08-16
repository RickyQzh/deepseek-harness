# dsh-sandbox

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的失败即关闭的沙箱模式、可写根、提权，以及本地 confine 包装。`SandboxMode` 使用 kebab-case（`read-only`、`workspace-write`、`danger-full-access`），且只命名文件效果。

`writable_roots` 在 `read-only` 与 `danger-full-access` 下为空；在 `workspace-write` 下是策略工作区根、`/tmp` 与平台临时目录去重后的规范路径集合。`canonical_path` 使用 `std::fs::canonicalize`，解析失败时返回输入的原样拼写。

`LocalSandboxProvider::confine` 用 bwrap、Landlock 或 Seatbelt 包装 argv，且从不返回原始 argv。Linux 按顺序探测 bwrap 再探测 Landlock；Darwin 将 Seatbelt 作为唯一候选项、不经探测即选中。空链或全部探测不可用时为 `SANDBOX_UNAVAILABLE`。Landlock 启动器路径从仓库布局解析（`native/landlock-run/packages/linux-{arch}/bin/landlock-run`，然后是匹配的 `node_modules` 包），从不读取进程环境变量。不选择 Windows ACL。

`validate_escalation_args` 要求 `sandbox_permissions` 与非空 justification 成对出现。`approve_escalation` 按顺序失败即关闭：并非严格更宽、没有审批服务、没有 agent（智能体），然后才是审批结果。`SandboxPolicy::confined` 拒绝 `danger-full-access`。缺少后端时为 `SANDBOX_UNAVAILABLE`，并使用 TypeScript 拒绝段落（含 Windows ACL 子句）。

模型可见标记：`[sandbox: file access denied under {mode} mode]` 与 `[sandbox: escalation available — retry this exact {subject} once with sandbox_permissions (the narrowest wider mode that suffices) + justification; the approval prompt asks the user]`。

## 已知限制与暂缓事项

- resolve 不折叠 `sandbox/mode` 会话事件；解析出的模式即构造时的默认模式。
- 不选择 Windows ACL；注入的 `win32` 平台得到空链并失败即关闭。
- runner 选择在提供方生命周期内缓存；安装或修复 runner 需要新建提供方。
