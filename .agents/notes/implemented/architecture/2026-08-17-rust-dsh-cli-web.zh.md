# Agent Note: Run dsh web on the Rust host

Status: implemented

[English](2026-08-17-rust-dsh-cli-web.md) | 中文

## 问题

[在 dsh-host 中组合 Rust dsh web 插件图](2026-08-17-rust-web-plugin-graph.md) 已交付 `WEB_YAML` 与 `register_host_plugins`，但捆绑 YAML 省略 frontend-static 的 `dist` 与 client-modules 的 `dir`，因此在调用方给出目录之前，启动该文件会加载失败。`dsh-cli` 把 argv[1] 为 `web` 以及 `--profile web` 都当作未实现，所以没有进程去组装该图。

## 决策

`dsh-cli` 把 `dsh web` 解析为 `--profile web` 的别名，得到 `ParsedCli::Web(WebLaunch)`。两种形式都不需要位置参数任务。`--port` 默认 3080（`0` 由操作系统分配）。`--host` 为省略或 `127.0.0.1`；任何其他值（包括 `0.0.0.0`）都是 `CliError::Usage`，其消息包含 `is intentionally not supported yet for safety: it would expose remote code execution to the network; use 127.0.0.1 instead`。`--trusted-host` 可重复。`--dist` 可选。argv[1] 为 `plugin` 仍为未实现。无头解析、未设置 `DSH_CORDIS_CONFIG` 时的 `MINIMAL_YAML`、`headless-ok` 以及 jsonrpc 二进制保持不变；那些路径不调用 `register_host_plugins`。

`run_web` 调用 `ensure_persist_env`，在 `$DSH_CORDIS_CONFIG` 已设置且非空时加载该文件，否则加载 `dsh_host::WEB_YAML`，解析 dist（`--dist`，否则 `DSH_WEB_DIST`，否则 `cwd/apps/web/dist`；目录缺失时失败，消息含 `dist`），解析客户端包目录（`DSH_CLIENT_PACKAGES` 已设置且非空时用它，否则用空的临时目录），提供 `appExit`、空的 `cmdlineArgs` 与 `WebIo::stdio()`，并注册 spine、execution、base 与 `register_host_plugins`（不注册 headless）。用户 `--patch` 文档先应用。最后一份生成的 overlay 按插件 `name` 替换 `@deepseek-ai/dsh-host-webserver`（`host` 为 `127.0.0.1`、`port`，以及非空时的 `trustedHosts`）、`@deepseek-ai/dsh-host-frontend-static`（`dist`）与 `@deepseek-ai/dsh-client-modules`（`dir`）的配置。必须按 `name` overlay，因为捆绑的 `WEB_YAML` 行没有 `id`。进程等待 `appExit`；SIGTERM 杀死监听器。

host-webserver 读取可选的 `trustedHosts` 并提供该服务；`web-app` 把它注入 `HostState`。`dsh-cli` 依赖 `dsh-host`。`dsh-host` 不依赖 `dsh-cli`。`dsh-agent` 不依赖 `dsh-host`。

## 备选方案

**针对 `WEB_YAML` 使用按 id 的 `--patch` overlay。** 不予采用：那些行没有 `id`，`apply_entry_patches` 会失败即响；按 `name` overlay 匹配 CLI 自己生成的 YAML。

**默认扫描 `packages/client`。** 不予采用：缺少 `lib/client.js` 会加载失败并给出构建说明；默认空临时目录让 `cargo test -p dsh-cli` 不依赖 Playwright 和那棵树。

**绑定 `--host 0.0.0.0`。** 不予采用：仅 loopback 的 `HostBind` 与用法错误句子把远程代码执行挡在网络之外。

**只由 `dsh-cli` 以上下文服务提供 `trustedHosts`，webserver 配置保持封闭。** 不予采用：overlay 已经点名 webserver 配置键；让该插件读取 `trustedHosts` 键，才能把该 flag 留在组合文档里。

## 影响

`cargo test -p dsh-cli --offline` 把 `dsh web --port 0` 解析为 `WebLaunch.port == 0`，把 `--profile web` 当作端口 3080 的 web，用安全句子拒绝 `--host 0.0.0.0`，保持 `plugin` 与未知 profile 为未实现，并且 `dsh_web_bin_prints_ready_and_host_describe` 在未设置 `DSH_CORDIS_CONFIG` 时 spawn `CARGO_BIN_EXE_dsh` `web --port 0`，等待 `dsh web: http://127.0.0.1:<port>`，并断言 `POST /api/host.describe` 为 HTTP 200 且 `canOpenPath` 为 false。

## 相关

捆绑图见[在 dsh-host 中组合 Rust dsh web 插件图](2026-08-17-rust-web-plugin-graph.md)。无头默认 YAML 见[第 6 阶段产品插件](2026-08-16-rust-dsh-base-plugins.md)。
