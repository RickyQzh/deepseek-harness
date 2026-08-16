# dsh-skill

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的 skill（技能）提供方注册表、本地文件系统提供方，以及面向模型的 `skill` 工具。一个 crate 注册三个 YAML 名称：`@deepseek-ai/dsh-skill`、`@deepseek-ai/dsh-skill-filesystem` 与 `@deepseek-ai/dsh-tool-skill`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

`register_skill` 以 `SkillRegistry` 提供 `skills`。`inject::<SkillRegistry>()` 得到 `Arc<SkillRegistry>`，因此 `register_provider`、`list` 与 `get` 在内部 mutex 上取 `&self`。重复的提供方名称会失败。同名 skill 先按更低 rank 再按提供方注册顺序取胜；`list` 返回按名称排序的获胜摘要。`get` 把该获胜摘要（含发现来源 `source`）覆盖到已加载正文上。本阶段只有一层全局层（没有 isolate realm）。skill 名称匹配 `/^[a-z0-9]+(?:-[a-z0-9]+)*$/`。

`register_skill_filesystem` 在该注册表上登记 `FilesystemSkillProvider`。默认根目录为项目 `.dsh/skills`（rank 100）、项目 `.agents/skills`（200）、用户 `$DSH_HOME/skills` 或 `~/.dsh/skills`（400，跳过 `.system`）、用户 `$DSH_AGENTS_HOME/skills` 或 `~/.agents/skills`（500），以及 rank 为 `BUNDLED_SKILL_RANK`（600）的 `$DSH_BUNDLED_SKILL_DIR`。布局为 `<name>/SKILL.md` 或 `<name>.md`，带 YAML frontmatter（必填 `name` 与 `description`）。缺失的根目录和格式错误的文件会警告并跳过。不实现文件监视。

`register_tool_skill` 注册 `skill` 工具（`{ name: string }`），并在该工具已注册且至少存在一个模型可调用 skill 时，于首次 `agent/pre-step` 前置一条持久化 `user/message`，其 `MessageSource::SkillCatalog { form: "catalog", update, entries }`，然后调用 `next()`。目录描述会规范化空白，并按 `catalogDescriptionMaxLength` 截断（默认 500，最小 3）。工具结果是 TypeScript 的 `<skill_content>` 包装（转义后的 `name` 属性、提供方资源提示、原文正文）。无法解析的名称返回工具错误。本阶段不实现用户手势 `/name` 注入。

## 模型体验

`skill` 工具结果和持久化目录消息对模型可见。正文仅在调用 `skill` 之后加载。目录行是 `<available_skills>` 内的 `- \`{name}\`: {description}`。

#### KV Cache 影响

初始目录是持久化的用户角色前缀。之后的工具结果追加已加载正文。

## 已知限制与暂缓事项

- 不实现文件监视 / 目录热刷新；每次 `list`/`get` 都会发现。
- 不实现 isolate/preset skill 层；注册表只有一层全局层。
- 不注册用户显式 `/name` 手势注入。
- 不移植 `dsh-skill-badge`。
- 本阶段不把这些插件挂入 `base.cordis.yml`。
