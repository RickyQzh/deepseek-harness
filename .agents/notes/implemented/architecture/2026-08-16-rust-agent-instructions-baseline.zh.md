# Agent Note: Rust agent-instructions baseline inject and JSONL resume

Status: implemented

[English](2026-08-16-rust-agent-instructions-baseline.md) | 中文

## Problem

TypeScript 的 [workspace-context 插件](../feature/2026-06-24-workspace-context.md) 把 `AGENTS.md` / `CLAUDE.md` 注入为已记录的 `user/message`，且 `source.kind == "agent-instructions"`。[Rust 重写](../../proposed/architecture/2026-08-14-rust-rewrite.md) 的第 5 阶段需要在 Rust 宿主上具备该基线，以便 headless 恢复能在离线编辑文件后重新匹配；此时还不把该插件挂入 `register_spine_plugins` 或 `base.cordis.yml`，也不引入 `dsh-agent-loop` → `dsh-agent-instructions` 的 crate 环。

TypeScript 的 `workspaceBaselineIdentity` 只哈希发现配置。文件内容漂移稍后由 `state.ts` 对账检测。本 crate 没有 `state.rs`，因此仅配置身份会把离线的 `AGENTS.md` 编辑当成匹配并跳过新基线。

## Decision

`dsh-agent-instructions` 注册 YAML 名称 `@deepseek-ai/dsh-agent-instructions`（仅在 `dsh-boot` 中的 `PLUGIN_AGENT_INSTRUCTIONS`）。`maxBytes` 必填；未知键会在加载时失败。发现过程从会话 `cwd` 向上走到 `projectRootMarkers`（默认 `.git`），在每个目录加载所有已存在的候选（先基线再 `.local`），以及 `$DSH_HOME/AGENTS.md`（display path 为 `$DSH_HOME/AGENTS.md`）。提供了 `fs` 时，读取走 `LocalFileSystem::read_text`；单元测试可以使用 `std::fs`。

`agent/pre-step` 监听器通过 `CompactionScope` 读取实时会话（仅同步 `with_session`）。在 `Enter { messages }` 且 vec 非空时，若可见表层没有这样的基线、或 `baselineIdentity` 不匹配，则前置一条用户角色的 `MessageSource::AgentInstructions`（`form: "instructions"`，`baseline: true`），然后始终调用 `next(decision)`。空的 `Enter` 保持不变，以免凭空造出只有 instructions 的一步。包装字符串从 TypeScript `render.ts` 原样拷贝：`<system-reminder>` 框架、`WORKSPACE_CONTEXT_INTRO`，以及 `Instructions from: {displayPath}\n\n{content}`。转义把 `</system-reminder>` 替换为 `<\/system-reminder>`。

`baselineIdentity` 是发现配置 **以及** 每个已加载文件 `{path, digest}`（文件字节的 SHA-1）的 JSON 摘要，因此无需对账也能让离线编辑失配。`MessageSource::AgentInstructions` 通过现有的 `rename_all = "kebab-case"` 序列化为 `kind: "agent-instructions"`。`SESSION_FORMAT_VERSION` 仍为 `0`。

`JsonlSessionStore::load` 读取 `path_for(id)`，再 `decode_session_log`，然后 `Session::from_events`。`AgentRegistry::resume` 要求 `options.session_id == session.id()`，在该 id 已注册时返回现有句柄，否则用已加载会话构建 `LoopAgent::new`（不是 `Session::new`）。Headless YAML `resumeSessionId` 会先 load 再 resume，然后仍然 followup 命令行任务。

## Alternatives considered

**移植 TypeScript 仅配置的 `workspaceBaselineIdentity`。** 否决：没有 `state.ts` 对账时，离线编辑 `AGENTS.md` 会保持同一身份并跳过注入。文件 digest 才是本 crate 拥有的恢复检查。

**像 TypeScript 那样把基线插到已认领消息之后。** 本阶段否决：`Enter { messages }` 就是日志，绑定的注入规则是前置然后 `next()`。已认领的用户文本仍会进入，不会被丢弃。

**在 `from_events` 旁边再加 `Session::from_replay`。** 否决：`Session::from_events` 已经会重建表层。

**从 `register_spine_plugins` 注册或挂入 `base.cordis.yml`。** 否决：spine 组合仍是 [Spine YAML 插件](2026-08-16-spine-plugins-in-dsh-agent.md) 中的封闭列表；本 crate 在后续任务之前按 YAML 选择加入。

**让 `dsh-agent-loop` 依赖本 crate。** 否决：循环拥有 waterfall（瀑布式事件）类型；本 crate 是监听器。反向边会在循环进入 spine 图后成环。

## Consequences

需要工作区指令的产品在 YAML 中列出 `@deepseek-ai/dsh-agent-instructions` 并显式给出 `maxBytes`。文件系统工具 touch 之后的动态对账、嵌套后代发现，以及完整的 TypeScript 省略/截断渲染属于后续工作。`cargo test -p dsh-agent-instructions --offline` 钉住首次请求注入与离线编辑后恢复。`cargo test -p dsh-session-persist --offline load_round_trips` 与 `cargo test -p dsh-headless --offline resume` 钉住 JSONL load 与 YAML `resumeSessionId`。

## Related

TypeScript 插件的产品行为见 [workspace context](../feature/2026-06-24-workspace-context.md)、[全量加载去重](../feature/2026-07-21-instruction-load-all-dedup.md) 与 [本地覆盖层](../feature/2026-07-21-local-instruction-overlay.md)。
