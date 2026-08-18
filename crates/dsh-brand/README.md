# dsh-brand

English | [中文](README.zh.md)

Compile-time branded wrappers around owned strings for the Rust host. Owning crates define an empty tag type and alias the product id; this crate does not name `SessionId`, `CallId`, or other product ids.

```rust
use dsh_brand::Branded;

struct SessionIdTag;
type SessionId = Branded<SessionIdTag>;

let id = SessionId::new("sess-1");
assert_eq!(id.as_str(), "sess-1");
```

Comparison, hashing, display, and logging use the inner string. The TypeScript package `@deepseek-ai/dsh-brand` remains the type-only primitive for the TypeScript tree.

## Known Limitations and Deferred Work

- Serde and JSON codecs belong in persistence and wire crates, not here.
