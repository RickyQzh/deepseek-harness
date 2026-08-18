# dsh-session

English | [中文](README.zh.md)

Append-only session log types for the Rust host: branded `SessionId` / `MessageId` / `CallId`, `SESSION_FORMAT_VERSION = 0`, the closed first-party `SessionEvent` enum, surface fold, `derive_messages`, request-header fold, interrupted-turn repair, and packed chunk-row encoding.

`Session::set_append_sink` installs an optional observer invoked after each successful `append` (`seq` already assigned).

This crate does not depend on `dsh-compose` or `dsh-kernel`. JSONL framing and zstd live in `dsh-session-persist`. Product ids are local newtypes around `dsh_brand::Branded`; `dsh-brand` does not name them.

## Known Limitations and Deferred Work

- SQLite `SCHEMA_VERSION = 15` is a later persist backend, not this crate.
- `Session` does not auto-append `session/end-seed`; that is store-create behavior.
