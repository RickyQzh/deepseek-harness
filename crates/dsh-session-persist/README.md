# dsh-session-persist

English | [中文](README.zh.md)

JSONL is the product default (`session.jsonl` / `session.jsonl.zstd`). This crate encodes the `type: "session"` header line, event lines, packed chunk rows, and concatenated checksummed zstd frames. It depends on `dsh-session`, not on compose.

`JsonlSessionStore` writes uncompressed unpacked JSONL to `{root}/{sessionId}/session.jsonl` (`SessionId::as_str()`, no `--<project>--` segment) so the file matches Python SDK and jsonrpc fixtures; `flush` may rewrite the whole file. `from_env` uses `DSH_SESSION_ROOT` when that variable is set and non-empty, otherwise `{DSH_HOME}/sessions`.

`plugin::register` provides `sessions` as `JsonlSessionStore` from `from_env`, or `with_root` when config `root` is a string.

`parse_header_record` runs `refuse_foreign_format_version` on the parsed JSON before `HeaderLine` validation, so a newer format is refused as unsupported rather than corrupt. Retired `sandboxMode` / `approvalPolicy` fields are corruption (`session header uses retired policy baseline fields`). `delegationDepth` is always written; a missing in-memory value becomes `0`.

Compression uses the `zstd` 0.13 crate wrapping libzstd. crate `zstd` 0.13 is MIT OR Apache-2.0; `zstd-sys` bundles facebook/zstd dual-licensed BSD-3-Clause OR GPL-2.0; this workspace takes the BSD grant. Frames are written with `Encoder::include_checksum(true)`.

## Known Limitations and Deferred Work

- SQLite is a later persist backend; this crate does not depend on rusqlite.
- Surface, derive, and repair stay in `dsh-session`.
