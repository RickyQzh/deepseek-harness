# Agent Note: Rust toolchain pin, source-vs-artifact split, and CI lane

Status: proposed

English | [中文](2026-08-14-rust-tooling-and-gates.zh.md)

## Problem

The repository has a Cargo workspace beside the TypeScript tree. Without a rustc pin, a fmt/clippy/test split, and a separate Actions workflow, Rust work either rides the Node 24 lane or has no required check.

## Proposal

- Pin rustc **1.85.0** via `rust-toolchain.toml` (`edition = "2024"`, workspace `rust-version = "1.85"`). Do not use `stable` as the CI channel.
- Source plane: `cargo test`, `cargo clippy --all-targets`, `cargo fmt --check` against `crates/*/src` on a clean tree.
- Artifact plane: `cargo build --release` / the installed host binary. Snapshot and Python SDK tests that claim to exercise the shipping product must run that binary, never `cargo test` in-process, once a bin exists.
- CI: `.github/workflows/rust.yml` (`name: Rust`, job `fmt clippy test`), independent of `ci.yml`. Path filters: `crates/**`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`, `.github/workflows/rust.yml`. Maintainers mark the `Rust / fmt clippy test` check required in branch protection.
- llvm-cov becomes required when a crate with branches other than `dsh-brand` lands. cargo-deny / copyleft scanning for crates is a follow-up when the first third-party crate is added.
- Node 24 lanes in `ci.yml` stay until the last TypeScript host package they cover is deleted.

Cite [the rewrite note](../architecture/2026-08-14-rust-rewrite.md) rather than restating keep-or-drop.

## Alternatives considered

- **Fold cargo into `scripts/run-gates.ts` / `ci.yml` node-24** — rejected: the Node lane already saturates hosted runners; landlock-run already uses a sibling workflow for the same reason.
- **Channel = stable** — rejected: fmt/clippy diagnostics would drift without a lockstep PR.
- **llvm-cov from day one** — rejected until a crate has branches worth covering; `dsh-brand` is a newtype.
- **Leave Cargo.lock untracked** — rejected: the workspace will ship a binary; lockfile is the artifact-plane input.

## Acceptance criteria

- `rust-toolchain.toml` names `1.85.0` and includes `rustfmt` and `clippy`.
- `cargo test --workspace` is green for `dsh-brand`.
- `.github/workflows/rust.yml` does not edit Node jobs.
- `docs/development.md` states the rustc source-vs-artifact split.
- `docs/architecture.md` is unchanged.

## Risks

A path-filtered required check is skipped on PRs that do not touch `crates/**`, so a later accidental `Cargo.toml` edit on a docs PR could surprise. Pinning 1.85.0 rather than rolling stable makes edition upgrades an explicit PR.
