# dsh-agent-instructions

[English](README.md) | 中文

面向 Rust 宿主的工作区指令加载器。`plugin::register` 读取必填 YAML `maxBytes`（dsh-base 使用 65536），以及可选的 `dshHome`、`projectRootMarkers`（默认 `[".git"]`）、`maxSourceBytes`（默认 1048576）、`instructionFileCandidates`（默认 `AGENTS.md`、`CLAUDE.md`）和 `localInstructionFileCandidates`（默认 `AGENTS.local.md`、`CLAUDE.local.md`）。未知键和缺失的 `maxBytes` 会在加载时失败。YAML 名称为 `@deepseek-ai/dsh-agent-instructions`。本 crate 不加入 `register_spine_plugins`。

发现过程从会话 `cwd` 向上走到 `projectRootMarkers`，在每个目录加载所有已存在的候选（先基线再 `.local` 覆盖层），以及用户全局 `$DSH_HOME/AGENTS.md`（`displayPath` 为 `$DSH_HOME/AGENTS.md`）。`agent/pre-step` 监听器在可见表层没有 `baseline: true` 的 `MessageSource::AgentInstructions`、或 `baselineIdentity` 不匹配时，把用户角色基线（`form: "instructions"`，`baseline: true`）前置到 `Enter { messages }`；这些消息就是日志。包装文本沿用 TypeScript 的 `<system-reminder>` 框架、`WORKSPACE_CONTEXT_INTRO` 以及 `Instructions from: {displayPath}` 各节。

`JsonlSessionStore::load` 与 `AgentRegistry::resume` 从未压缩 JSONL 重建会话。Headless YAML `resumeSessionId` 会先 load 再 resume，然后仍然 followup 命令行任务。

## 已知限制与暂缓事项

- 文件系统工具 touch 之后的动态对账、嵌套后代发现，以及完整的字节预算省略/截断渲染属于后续工作；本 crate 注入完整基线，或在身份不匹配时注入替换基线。
- 本阶段不把该插件挂入 `base.cordis.yml`。
