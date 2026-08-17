# dsh-host

English | [中文](README.zh.md)

GUI host crate for the DeepSeek Harness Rust process. This crate will own the axum listener in later tasks. It currently exports the `/api` trust fence (`is_trusted_api_request`, `assert_trusted_authority`, `is_loopback_hostname`) and the privileged-method set (`is_privileged_method`, `privileged_requires_loopback`) only.

`is_loopback_hostname` takes a hostname (port already stripped): `localhost`, `[::1]`, or IPv4 127/8. `is_trusted_api_request` takes the Host header (`host[:port]`), Origin, `sec-fetch-site`, and `trustedHosts`. Missing or unparsable Host is untrusted. Host must be loopback or a declared trusted authority. `sec-fetch-site: cross-site` is refused. Absent Origin is trusted after Host. Origin `"null"` is refused. Origin host must equal Host host.

`assert_trusted_authority` requires a bare `host` or `host:port` that survives WHATWG-equivalent parse unchanged (lowercase compare). Path, userinfo, whitespace, a dangling colon, and non-canonical hosts such as `0x7f.0.0.1` fail loudly as `TrustError`.

Privileged dotted methods match the TypeScript `PRIVILEGED_METHODS` set. `privileged_requires_loopback` is true iff the Host header parses to a loopback hostname. Privileged methods still require loopback even when `trustedHosts` would pass the outer fence.

This crate does not listen, register plugins, or serialize RPC. It does not depend on `dsh-rpc`.

## Known Limitations and Deferred Work

- The axum listener, static SPA, `__DSH_BOOT__`, `/plugins`, unary dispatch, and WebSocket downlinks are not in this crate yet.
- Plugin YAML names for the web composition live as string constants on `dsh-boot`; this crate does not register them yet.
