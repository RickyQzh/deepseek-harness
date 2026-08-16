# dsh-agent-instructions

English | [中文](README.zh.md)

Workspace instruction loader for the Rust host. `plugin::register` reads required YAML `maxBytes` (dsh-base uses 65536) plus optional `dshHome`, `projectRootMarkers` (default `[".git"]`), `maxSourceBytes` (default 1048576), `instructionFileCandidates` (default `AGENTS.md`, `CLAUDE.md`), and `localInstructionFileCandidates` (default `AGENTS.local.md`, `CLAUDE.local.md`). Unknown keys and a missing `maxBytes` fail load. YAML name `@deepseek-ai/dsh-agent-instructions`. This crate is not added to `register_spine_plugins`.

Discovery walks from the session `cwd` up through `projectRootMarkers`, loads every existing candidate per directory (base then `.local` overlays), and the user-global `$DSH_HOME/AGENTS.md` (`displayPath` `$DSH_HOME/AGENTS.md`). An `agent/pre-step` listener prepends a user-role `MessageSource::AgentInstructions` baseline (`form: "instructions"`, `baseline: true`) onto `Enter { messages }` when the visible surface has no such baseline or `baselineIdentity` mismatches; those messages are the log. Wrapper text is the TypeScript `<system-reminder>` frame, `WORKSPACE_CONTEXT_INTRO`, and `Instructions from: {displayPath}` sections.

`JsonlSessionStore::load` plus `AgentRegistry::resume` rebuild a session from uncompressed JSONL. Headless YAML `resumeSessionId` loads then resumes before the cmdline followup.

## Known Limitations and Deferred Work

- Dynamic reconciliation after filesystem tool touches, nested descendant discovery, and the full byte-budget omit/truncate renderer are later work; this crate injects the complete baseline or a replacing baseline on identity mismatch.
- The plugin is not mounted in `base.cordis.yml` in this phase.
