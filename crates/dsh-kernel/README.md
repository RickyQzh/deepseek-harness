# dsh-kernel

English | [中文](README.zh.md)

Context, Fiber lifecycle, named services, effects, isolate realms, and the in-process event bus for the Rust host. This crate keeps Cordis semantics (named services, inject-wait, reverse-order dispose, waterfall `next()`, isolate realms) and does not implement Proxies, declaration merging, or `!!js`.

Fiber states are PENDING → LOADING → ACTIVE | FAILED | UNLOADING → DISPOSED. `Context::plugin` mounts a child fiber; `await_ready` settles on Active or returns the setup error. The event bus is implemented here; `dsh-events` re-exports it.

## Known Limitations and Deferred Work

- v1 plugins are trusted in-process Rust crates listed in a profile manifest; this crate does not load cdylibs.
- Logging stays with the caller; this crate does not depend on `tracing`.
