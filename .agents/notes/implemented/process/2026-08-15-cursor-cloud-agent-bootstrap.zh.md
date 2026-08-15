# Agent Note: 记录 Cursor Cloud Agent 的引导安装

Status: implemented

[English](2026-08-15-cursor-cloud-agent-bootstrap.md) | 中文

## Problem

本仓库的 Cursor Cloud Agent 工作区启动时没有 `node_modules`，且默认 `node` 经常低于根目录 `engines.node` 范围 `^22.19.0 || >=24.0.0`。同一工作区会把 `core.hooksPath` 固定到 Cursor agent hooks，因此普通 `pnpm install` 会在 [`install-lefthook.mjs`](../../../../scripts/install-lefthook.mjs) 中失败：该继承路径并非 Lefthook 所有。此前没有一份已记录的安装配方，能在这两条约束下得到可用的检出目录。

## Decision

[`.cursor/environment.json`](../../../../.cursor/environment.json) 是 Cloud Agent 的安装配方。存在 nvm 时优先使用 nvm 的 Node 22，避免 `/exec-daemon/node` 的 22.14 抢占 `PATH`；通过 Corepack 启用仓库固定的 `pnpm@11.7.0`；并运行 `CI=true pnpm install`。`CI=true` 是自动化作业已使用的 Lefthook 安装器空操作（[worktree 本地 Lefthook](2026-07-27-worktree-local-lefthook.md)），因此 postinstall 仍会执行 `packages/subprocess/subprocess-local` 的 spawn-helper 修复，且不会替换 Cursor hooks。该文件没有 `start` 字段：Web UI 与文档站按需启动。当 `pnpm run typecheck` 成功退出时，搭建完成，与 [development.md](../../../../docs/development.md#first-time-setup) 一致。

## Alternatives considered

**设置 `DSH_LEFTHOOK_ALLOW_HOOKS_PATH_OVERRIDE=1`。** 这会让当前 worktree 改用 Lefthook，并丢掉继承来的 Cursor hooks，除非再通过 `lefthook.yml` 把它们串起来。Cloud Agent 会话需要那些继承 hooks。

**使用 `pnpm install --ignore-scripts`。** 这样能避开 Lefthook 拒绝，但也会跳过 spawn-helper 的 chmod 以及所有其他工作区 postinstall。

**只保留个人 draft 环境，不把配方写入仓库。** 数据库中的 draft 无法评审，也不能分享给本仓库后续的 Cloud Agent 运行。

**提高默认 Cloud snapshot 的 Node 版本，而不是让安装脚本优先使用 nvm。** 本仓库并不拥有 snapshot 目录，且当前 Cloud 镜像里已经有 nvm 的 Node 22。

## Consequences

Cloud Agent 安装变为可幂等、可评审。贡献者机器不受影响：它们仍在没有 `CI=true` 的情况下运行 leftover，并仍会拒绝非本仓库所有的继承 `core.hooksPath`。除非后续改动通过 `lefthook.yml` 串接 Cursor hooks，Cloud Agent worktree 不会获得仓库 Lefthook hooks。
