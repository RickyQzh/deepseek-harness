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

Comparison, hashing, display, logging, and JSON string serde use the inner string. The TypeScript package `@deepseek-ai/dsh-brand` remains the type-only primitive for the TypeScript tree.

## Known Limitations and Deferred Work

- Product ids are named in owning crates. `impl Serialize for Branded<LocalTag>` is orphan-illegal, so string serde lives on `Branded<B>` here.
