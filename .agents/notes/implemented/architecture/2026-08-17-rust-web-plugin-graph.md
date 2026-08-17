# Agent Note: Compose the Rust dsh web plugin graph in dsh-host

Status: implemented

English | [中文](2026-08-17-rust-web-plugin-graph.zh.md)

## Problem

Phase 7 already binds the loopback listener, unary `/api` carrier, WebSocket downlinks, and `GuiHandler`. Without a bundled YAML graph and host plugin registrations, nothing listens as a composed web app or prints the ready URL. Copying `HeadlessIo` by depending on `dsh-headless` would couple the GUI host to the one-shot runner. Putting `register_host_plugins` behind `dsh-cli` would leave `dsh-host` unable to boot itself in tests.

## Decision

`dsh-host` owns `WEB_YAML` (`web.cordis.yml`) and `register_host_plugins`. The YAML is the Phase 6 headless base rows without `headless-startup`, `headless-runner`, `headless-auto-approve`, and `sdk-jsonrpc-server`, plus workspace, settings, commands, `@deepseek-ai/dsh-host-webserver` (`127.0.0.1:3080`), `@deepseek-ai/dsh-host-frontend-static`, `@deepseek-ai/dsh-client-modules`, and `web-app` (`printUrl: true`). There is no `!!js`. User-approval `policy` is `ask`. The mock LLM row stays for keyless boot.

`register_host_plugins` registers only those seven names. It does not call `register_spine_plugins`, `register_execution_plugins`, or `register_base_plugins`. `dsh-host` does not depend on `dsh-cli` or `dsh-headless`. `dsh-headless` does not depend on `dsh-host`. `dsh-agent` does not depend on `dsh-host`.

Stdout capture is `WebIo` in `dsh-host` (`stdio`, `capture`, `stdout`, `take_stdout`). `web-app` uses inject `"webIo"` when present, otherwise `WebIo::stdio()`. After `serve`, it writes exactly `dsh web: http://127.0.0.1:<bound-port>\n` when `printUrl` is true (default) and provides `listeningHost`. Host-webserver provides `hostBind`; a host other than `127.0.0.1` fails load. Frontend-static provides `webDist` and fails load when `dist` is not an existing directory. Client-modules provides `clientPackages`; an empty directory is an empty graph.

The dotted map remains in [Phase 7 GUI RpcMethodMap](2026-08-17-rust-gui-rpc-method-map.md). The HTTP/WS wire freeze remains in [Freeze the Rust GUI host four-quadrant wire](../../proposed/architecture/2026-08-16-rust-gui-host-wire.md). `dsh web` is not in this crate.

## Testing

`web_yaml_rejects_js_tag_substring` asserts `WEB_YAML` contains no `!!js` and none of the four omitted headless/SDK names. `web_app_prints_ready_url_and_serves_index` boots spine + execution + base + host with port `0`, `printUrl: true`, a dist that contains `index.html`, and an empty client-packages directory, provides `WebIo::capture()`, then asserts stdout contains `dsh web: http://127.0.0.1:`, `GET /` is HTTP 200, and `listeningHost` shuts down. Persist paths are plugin `root` / `path` / `dir` config, not process env. `cargo test -p dsh-host --offline` keeps the earlier host tests.

## Alternatives considered

**Depend on `dsh-headless` and reuse `HeadlessIo`.** Rejected: the GUI host must not take a dependency on the one-shot runner, and `dsh-headless` must not depend on `dsh-host`.

**Have `register_host_plugins` also register spine, execution, and base.** Rejected: the CLI owns that assembly in a later task; this crate's registrar stays the seven host/product names so a caller can compose a smaller graph.

**Import `HeadlessIo` by duplicating the type into a shared util crate.** Rejected: one `WebIo` next to `web-app` is enough, and a shared IO crate would be a third package for two short types.

## Consequences

`WEB_YAML` omits frontend-static `dist` and client-modules `dir`, so a boot of the bundled file without those configs fails load. That is fail-loud until `dsh web` supplies directories. Workspace and settings still need `DSH_HOME` / `DSH_SESSION_ROOT` or `path` / `dir` config. The ready line is a stdout contract for supervisors; tests must capture `WebIo` rather than process stdout.
