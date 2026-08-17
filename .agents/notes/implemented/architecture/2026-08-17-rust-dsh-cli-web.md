# Agent Note: Run dsh web on the Rust host

Status: implemented

English | [中文](2026-08-17-rust-dsh-cli-web.zh.md)

## Problem

[Compose the Rust dsh web plugin graph](2026-08-17-rust-web-plugin-graph.md) ships `WEB_YAML` and `register_host_plugins`, but bundled YAML omits frontend-static `dist` and client-modules `dir`, so a boot of that file fails load until a caller supplies directories. `dsh-cli` treated `web` as argv[1] and `--profile web` as not implemented, so nothing assembled the graph as a process.

## Decision

`dsh-cli` parses `dsh web` as an alias of `--profile web` into `ParsedCli::Web(WebLaunch)`. Neither form requires a positional task. `--port` defaults to 3080 (`0` is OS-assigned). `--host` is omitted or `127.0.0.1`; any other value, including `0.0.0.0`, is `CliError::Usage` whose message contains `is intentionally not supported yet for safety: it would expose remote code execution to the network; use 127.0.0.1 instead`. `--trusted-host` is repeatable. `--dist` is optional. `plugin` argv[1] stays not implemented. Headless parse, `MINIMAL_YAML` when `DSH_CORDIS_CONFIG` is unset, `headless-ok`, and the jsonrpc bin stay unchanged; those paths do not call `register_host_plugins`.

`run_web` calls `ensure_persist_env`, loads `$DSH_CORDIS_CONFIG` when set and non-empty otherwise `dsh_host::WEB_YAML`, resolves dist (`--dist` else `DSH_WEB_DIST` else `cwd/apps/web/dist`; missing directory fails with a message containing `dist`), resolves client packages (`DSH_CLIENT_PACKAGES` when set and non-empty, else an empty temporary directory), provides `appExit`, empty `cmdlineArgs`, and `WebIo::stdio()`, and registers spine, execution, base, and `register_host_plugins` (not headless). User `--patch` documents apply first. A generated overlay last replaces config by plugin `name` for `@deepseek-ai/dsh-host-webserver` (`host` `127.0.0.1`, `port`, and `trustedHosts` when non-empty), `@deepseek-ai/dsh-host-frontend-static` (`dist`), and `@deepseek-ai/dsh-client-modules` (`dir`). Name overlay is required because bundled `WEB_YAML` rows have no `id`. The process waits on `appExit`; SIGTERM kills the listener.

Host-webserver reads optional `trustedHosts` and provides that service; `web-app` injects it into `HostState`. `dsh-cli` depends on `dsh-host`. `dsh-host` does not depend on `dsh-cli`. `dsh-agent` does not depend on `dsh-host`.

## Alternatives considered

**Id-targeted `--patch` overlays against `WEB_YAML`.** Rejected: those rows have no `id`, so `apply_entry_patches` would fail loud; name overlay matches the generated YAML the CLI owns.

**Scan `packages/client` by default.** Rejected: a missing `lib/client.js` fails load with a build instruction; the default empty temp directory keeps `cargo test -p dsh-cli` free of Playwright and of that tree.

**Bind `--host 0.0.0.0`.** Rejected: the loopback-only `HostBind` and the usage-error sentence keep remote code execution off the network.

**Provide `trustedHosts` only from `dsh-cli` as a context service and leave webserver config closed.** Rejected: the overlay already names webserver config keys; teaching that plugin the `trustedHosts` key keeps the flag in the composition document.

## Consequences

`cargo test -p dsh-cli --offline` parses `dsh web --port 0` as `WebLaunch.port == 0`, treats `--profile web` as web with port 3080, rejects `--host 0.0.0.0` with the safety sentence, keeps `plugin` and unknown profiles not implemented, and `dsh_web_bin_prints_ready_and_host_describe` spawns `CARGO_BIN_EXE_dsh` `web --port 0` with `DSH_CORDIS_CONFIG` unset, waits for `dsh web: http://127.0.0.1:<port>`, and asserts `POST /api/host.describe` is HTTP 200 with `canOpenPath` false.

## Related

The bundled graph is [Compose the Rust dsh web plugin graph](2026-08-17-rust-web-plugin-graph.md). Headless default YAML is [Phase 6 product plugins](2026-08-16-rust-dsh-base-plugins.md).
