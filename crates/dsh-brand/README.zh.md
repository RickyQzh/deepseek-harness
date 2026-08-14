# dsh-brand

[English](README.md) | 中文

为 Rust 宿主提供包住自有字符串的编译期品牌包装。所属 crate 定义空的 tag 类型，并为产品 id 建立别名；本 crate 不命名 `SessionId`、`CallId` 或其他产品 id。

```rust
use dsh_brand::Branded;

struct SessionIdTag;
type SessionId = Branded<SessionIdTag>;

let id = SessionId::new("sess-1");
assert_eq!(id.as_str(), "sess-1");
```

比较、哈希、显示和日志均使用内部字符串。TypeScript 包 `@deepseek-ai/dsh-brand` 仍是 TypeScript 树中的仅类型原语。

## 已知限制与暂缓事项

- Serde 与 JSON 编解码器属于持久化和协议 crate，不属于本 crate。
