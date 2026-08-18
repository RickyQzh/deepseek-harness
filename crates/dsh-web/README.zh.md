# dsh-web

[English](README.md) | 中文

面向 DeepSeek Harness Rust 宿主的 web 搜索 Service Definition。`plugin::register` 以 `WebRuntime` 提供 `web`。`inject::<WebRuntime>()` 得到 `Arc<WebRuntime>`，因此 `register_search_provider` 与 `search` 在内部 mutex 上取 `&self`。YAML 名称为 `@deepseek-ai/dsh-web`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

本阶段省略 fetch 类型、`register_fetch_provider` 与 `web_fetch`，因为按 URL 抓取存在 SSRF 风险。唯一操作是搜索。

选择在执行时解析，绝不依赖注册顺序。已配置但未注册的 id 失败为 `WEB_PROVIDER_CONFIGURED_MISSING`（`configured web provider "{id}" is not registered`）。已配置且已注册但 `available() == false` 失败为 `WEB_PROVIDER_CONFIGURED_UNAVAILABLE`。未配置 id 时，恰好一个可用提供方会被选中；多个可用提供方失败为 `WEB_PROVIDER_AMBIGUOUS`；零个失败为 `WEB_PROVIDER_UNAVAILABLE`。重复 id 失败为 `WEB_DUPLICATE_PROVIDER`（`a web provider with id "{id}" is already registered`）。`search` 将 `sources` 截断到 `max_results`，并在提供方超额返回时设置 `truncated`；当列表已在上限之内时，保留提供方自己的 `truncated` 标志。

`AbortFlag::is_aborted()` 失败为 `WEB_ABORTED`。`WebError` 的 Display 是消息；`code` 是稳定路由标记。

## 配置

| 键 | 默认 | 含义 |
|---|---|---|
| `searchProvider` | 省略 | 钉死搜索提供方 id。省略 = 恰好一个可用提供方时自动选择。 |

未知键会在加载时失败。

## 模型体验

间接通过 [`dsh-tool-web`](../dsh-tool-web/README.md)。本注册表不贡献工具 schema 或提示词。

#### KV Cache 影响

无直接失效。

## 已知限制与延后工作

- 本阶段省略 fetch（存在 SSRF 风险的 URL 抓取），留待后续阶段。
- 未实现提供方注销 / HMR disposer。
- 本阶段不把该插件挂入 `base.cordis.yml`。
