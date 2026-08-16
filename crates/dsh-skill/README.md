# dsh-skill

English | [中文](README.zh.md)

Skill provider registry, local filesystem provider, and model-facing `skill` tool for the DeepSeek Harness Rust host. One crate registers three YAML names: `@deepseek-ai/dsh-skill`, `@deepseek-ai/dsh-skill-filesystem`, and `@deepseek-ai/dsh-tool-skill`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

`register_skill` provides `skills` as a `SkillRegistry`. `inject::<SkillRegistry>()` yields `Arc<SkillRegistry>`, so `register_provider`, `list`, and `get` take `&self` with an interior mutex. Duplicate provider names fail. Same-name skills resolve by lower rank, then provider registration order; `list` returns the winning summaries sorted by name. This phase is one global layer (no isolate realms). Skill names match `/^[a-z0-9]+(?:-[a-z0-9]+)*$/`.

`register_skill_filesystem` registers a `FilesystemSkillProvider` on that registry. Default roots are project `.dsh/skills` (rank 100), project `.agents/skills` (200), user `$DSH_HOME/skills` or `~/.dsh/skills` (400, skips `.system`), user `$DSH_AGENTS_HOME/skills` or `~/.agents/skills` (500), and `$DSH_BUNDLED_SKILL_DIR` at `BUNDLED_SKILL_RANK` (600). Layout is `<name>/SKILL.md` or `<name>.md` with YAML frontmatter (`name` and `description` required). Missing roots and malformed files warn and skip. File watching is not implemented.

`register_tool_skill` registers the `skill` tool (`{ name: string }`) and prepends a durable `user/message` with `MessageSource::SkillCatalog { form: "catalog", update, entries }` at the first `agent/pre-step` when that tool is registered and at least one model-invocable skill exists, then calls `next()`. Catalog descriptions are whitespace-normalized and capped at `catalogDescriptionMaxLength` (default 500, minimum 3). The tool result is the TypeScript `<skill_content>` wrapper (escaped `name` attribute, provider resource hint, verbatim body). Unresolved names return a tool error. User-gesture `/name` injection is out of this phase.

## Model Experience

The `skill` tool result and the durable catalog message are model-visible. Bodies load only after a `skill` call. Catalog lines are `- \`{name}\`: {description}` inside `<available_skills>`.

#### KV Cache effect

The initial catalog is a durable user-role prefix. Later tool results append loaded bodies.

## Known Limitations and Deferred Work

- File watching / catalog hot-refresh is not implemented; discovery is per `list`/`get`.
- Isolate/preset skill layers are not implemented; the registry is one global layer.
- User-explicit `/name` gesture injection is not registered.
- `dsh-skill-badge` is not ported.
- The plugins are not mounted in `base.cordis.yml` in this phase.
