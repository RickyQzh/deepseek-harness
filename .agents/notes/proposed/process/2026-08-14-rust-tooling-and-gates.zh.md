# Agent Note: Rust 工具链钉住、源码平面与产物平面划分，以及 CI 车道

Status: proposed

[English](2026-08-14-rust-tooling-and-gates.md) | 中文

## 问题

仓库在 TypeScript 树旁有一个 Cargo workspace。若没有 rustc 钉住、fmt/clippy/test 划分，以及独立的 Actions 工作流，Rust 工作要么挤进 Node 24 车道，要么没有任何必需检查。

## 提案

- 通过 `rust-toolchain.toml` 钉住 rustc **1.85.0**（`edition = "2024"`，workspace `rust-version = "1.85"`）。不要把 `stable` 当作 CI 通道。
- 源码平面：对着干净树上的 `crates/*/src` 跑 `cargo test`、`cargo clippy --all-targets`、`cargo fmt --check`。
- 产物平面：`cargo build --release` / 已安装的宿主二进制。一旦二进制存在，声称检验交付产品的快照和 Python SDK 测试必须跑该二进制，而不是进程内的 `cargo test`。
- CI：`.github/workflows/rust.yml`（`name: Rust`，job `fmt clippy test`），独立于 `ci.yml`。路径过滤：`crates/**`、`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`rustfmt.toml`、`.github/workflows/rust.yml`。维护者在分支保护中把 `Rust / fmt clippy test` 检查标为必需。
- 当除 `dsh-brand` 以外、带分支的 crate 落地时，llvm-cov 成为必需。crate 的 cargo-deny / Copyleft 扫描是加入第一个第三方 crate 时的后续。
- `ci.yml` 中的 Node 24 车道保留到它们覆盖的最后一个 TypeScript 宿主包被删除。

引用[重写笔记](../architecture/2026-08-14-rust-rewrite.md)，不要复述保留或放弃表。

`AGENTS.md` 列出 cargo 源码平面命令，其 `verify-doc-budgets` 上限为 1950 词，因为这些行使该文件超过 1900。

## 曾考虑的替代方案

- **把 cargo 折进 `scripts/run-gates.ts` / `ci.yml` 的 node-24** — 否决：Node 车道已经占满托管 runner；landlock-run 已因同一理由使用兄弟工作流。
- **Channel = stable** — 否决：没有锁步 PR（Pull Request）时，fmt/clippy 诊断会漂移。
- **第一天就上 llvm-cov** — 否决：直到有 crate 带值得覆盖的分支；`dsh-brand` 是 newtype。
- **不跟踪 Cargo.lock** — 否决：workspace 将交付二进制；lockfile 是产物平面的输入。

## 验收标准

- `rust-toolchain.toml` 写明 `1.85.0`，并包含 `rustfmt` 与 `clippy`。
- `dsh-brand` 上 `cargo test --workspace` 为绿。
- `.github/workflows/rust.yml` 不改 Node job。
- `docs/development.md` 写明 rustc 的源码平面与产物平面划分。
- `docs/architecture.md` 保持不变。

## 风险

当 `crates/**`、`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`rustfmt.toml` 与 `.github/workflows/rust.yml` 均未改动时，Rust 车道会跳过。意外来自未列入过滤的新工具链文件（例如 `clippy.toml`），或工作流未运行时的跳过必需检查合并。钉住 1.85.0 而不是滚动 stable，使 edition 升级成为一次显式 PR。
