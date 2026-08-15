# Agent Note: Record Cursor Cloud Agent bootstrap

Status: implemented

English | [中文](2026-08-15-cursor-cloud-agent-bootstrap.zh.md)

## Problem

A Cursor Cloud Agent workspace for this repository starts without `node_modules` and often with a default `node` older than the root `engines.node` range `^22.19.0 || >=24.0.0`. The same workspace pins `core.hooksPath` to Cursor agent hooks, so a plain `pnpm install` fails in [`install-lefthook.mjs`](../../../../scripts/install-lefthook.mjs) when that inherited path is not Lefthook-owned. There was no recorded install recipe that produces a usable checkout under those two constraints.

## Decision

[`.cursor/environment.json`](../../../../.cursor/environment.json) is the Cloud Agent install recipe. It prefers nvm Node 22 when nvm is present so `/exec-daemon/node` 22.14 does not win `PATH`, enables the repository-pinned `pnpm@11.7.0` through Corepack, and runs `CI=true pnpm install`. `CI=true` is the existing Lefthook installer no-op used by automated jobs ([worktree-local Lefthook](2026-07-27-worktree-local-lefthook.md)), so postinstall still runs `packages/subprocess/subprocess-local` spawn-helper repair and does not replace Cursor hooks. The file has no `start` field: Web UI and the documentation site are started on demand. Setup is complete when `pnpm run typecheck` exits successfully, matching [development.md](../../../../docs/development.md#first-time-setup).

## Alternatives considered

**Set `DSH_LEFTHOOK_ALLOW_HOOKS_PATH_OVERRIDE=1`.** That opts the worktree into Lefthook and drops the inherited Cursor hooks unless they are chained through `lefthook.yml`. Cloud Agent sessions need those inherited hooks.

**Use `pnpm install --ignore-scripts`.** That avoids the Lefthook refusal but also skips the spawn-helper chmod and every other workspace postinstall.

**Keep a personal draft environment only, with no repository file.** A db-backed draft is not reviewable and is not shared with later Cloud Agent runs on this repository.

**Raise the default Cloud snapshot Node instead of teaching the install script to prefer nvm.** This repository does not own the snapshot catalog, and nvm Node 22 is already present on the current Cloud image.

## Consequences

Cloud Agent installs become idempotent and reviewable. Contributor machines are unchanged: they still run leftover without `CI=true` and still refuse an unowned inherited `core.hooksPath`. Cloud Agent worktrees do not receive repository Lefthook hooks unless a later change chains Cursor hooks through `lefthook.yml`.
