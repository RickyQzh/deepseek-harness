# dsh-user-approval

English | [中文](README.zh.md)

Fail-closed one-shot approval for the Rust host. `ApprovalService::request` requires an open turn; it appends `approval/asked`, runs the `approval/request` waterfall with payload `ApprovalQuestion` (default outcome `Unavailable`), then appends `approval/decided`. Policy `Never` still writes that pair but returns `Rejected` without calling answerers. An idle `request` returns `ApprovalError::Idle` and appends nothing.

YAML `@deepseek-ai/dsh-user-approval` config is `{ policy?: "ask"|"never" }` with default `"ask"`; unknown policy strings fail at load. YAML `headless-auto-approve` registers a terminal waterfall listener that returns `AllowedOnce` without `next()`. `plugin::register` and `plugin::register_auto_approve` live in this crate; `dsh-boot` does not call them. When `ctx.get::<ApprovalService>("approval")` is present, the agent plugin installs that service as the tools `Approver`.

`ApprovalOutcome` is the `dsh-tools` enum. Deny texts for `Rejected`, `Cancelled`, and `Unavailable` stay the TypeScript strings already in `dsh-tools`. See the [approval-seam Agent Note](../../.agents/notes/implemented/feature/2026-07-06-approval-seam.md).

## Known Limitations and Deferred Work

- Requests are valid only inside an open turn; a durable out-of-turn approval workflow is deferred.
- Only one-shot grants exist: `AllowedOnce` with no remembered rule or grant store; session policy is only `Ask` / `Never`.
- Runtime-context policy sentences and live `setPolicy` agent notices are not mounted here.
- The waterfall payload is `ApprovalQuestion`; answerers receive session id, optional call id, tool name, and the current `ApprovalOutcome`.
- This crate never prompts a human; `headless-auto-approve` is the unattended grant.
