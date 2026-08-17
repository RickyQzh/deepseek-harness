# dsh-host

English | [中文](README.zh.md)

GUI host crate for the DeepSeek Harness Rust process. This crate will own the axum listener in later tasks. It exports the `/api` trust fence (`is_trusted_api_request`, `assert_trusted_authority`, `is_loopback_hostname`), the privileged-method set (`is_privileged_method`, `privileged_requires_loopback`), and pure file/boot helpers for the SPA dist, `/plugins` bundles, and `window.__DSH_BOOT__`.

`is_loopback_hostname` takes a hostname (port already stripped): `localhost`, `[::1]`, or IPv4 127/8. `is_trusted_api_request` takes the Host header (`host[:port]`), Origin, `sec-fetch-site`, and `trustedHosts`. Missing or unparsable Host is untrusted. Host must be loopback or a declared trusted authority. `sec-fetch-site: cross-site` is refused. Absent Origin is trusted after Host. Origin `"null"` is refused. Origin host must equal Host host.

`assert_trusted_authority` requires a bare `host` or `host:port` that survives WHATWG-equivalent parse unchanged (lowercase compare). Path, userinfo, whitespace, a dangling colon, and non-canonical hosts such as `0x7f.0.0.1` fail loudly as `TrustError`.

Privileged dotted methods match the TypeScript `PRIVILEGED_METHODS` set. `privileged_requires_loopback` is true iff the Host header parses to a loopback hostname. Privileged methods still require loopback even when `trustedHosts` would pass the outer fence.

`serve_spa` answers one URL path from a dist directory: `..` or NUL is 403; a missing file or a directory falls back to `index.html` 200 after boot injection. `serve_plugin_js` reads `{root}/{package_name}/lib/client.js` (404 if missing) with `Cache-Control: no-cache`; `serve_plugin_source_map` uses `{path}.map` with the same rules.

`scan_client_packages` reads each `{dir}/*/package.json` and takes the nested `dsh.client` object (not a top-level `"dsh.client"` key). Packages with no `dsh` object are skipped. `platform == "web"` requires `lib/client.js` or fails as `HostError` whose text contains `pnpm run build` or `client bundle not found`. `immediately` defaults to false; `inject` is an optional string array. Malformed `dsh` / `dsh.client` fails loud.

`inject_boot_manifest` inserts `<script>window.__DSH_BOOT__ = {…}</script>` immediately before `</head>` (any case) or prepends. After JSON serialize, every `<` is replaced with `\u003c`. Each entry `url` is `/plugins/<id>/client.js?rev=<hex>`. Entry `rev` is the lowercase hex SHA-256 of that package's `lib/client.js` bytes. Graph `rev` is the lowercase hex SHA-256 of the UTF-8 concatenation of `id` then `rev` for each entry sorted by `id`.

This crate does not listen, register plugins, or serialize RPC. It does not depend on `dsh-cli`, `dsh-headless`, or `dsh-rpc`. `dsh-agent` must not depend on this crate.

## Known Limitations and Deferred Work

- The axum listener, unary `/api` dispatch, and WebSocket downlinks are not in this crate yet.
- Plugin YAML names for the web composition live as string constants on `dsh-boot`; this crate does not register them yet.
