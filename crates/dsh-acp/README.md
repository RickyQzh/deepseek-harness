# dsh-acp

English | [中文](README.zh.md)

ACP stdio adapter for the DeepSeek Harness Rust host. This crate is not the SDK JSON-RPC server (`dsh-sdk-jsonrpc-server`). Framing is NDJSON JSON-RPC 2.0, not LSP Content-Length. Stdout carries those ACP frames only.

`register` mounts YAML `@deepseek-ai/dsh-acp` (`dsh_boot::PLUGIN_ACP`). `register_acp_plugins` calls `register`. Plugin setup returns `Ok(())` after rejecting unknown config keys and does not serve stdio. The kernel service name is `acpServer`. This crate does not depend on `dsh-sdk-protocol`, `dsh-sdk-jsonrpc-server`, `dsh-base`, `dsh-host`, `dsh-cli`, `dsh-headless`, or `dsh-subagent`. `dsh-agent` must not depend on this crate.

## Known Limitations and Deferred Work

- Session load, list, resume, delete, and fork are unsupported.
- MCP, PTY, and LSP are unsupported.
- Continuable descendant drain is unsupported.
- Named snapshot subset is handshake, reject-extra-dirs, and text-turn only.
