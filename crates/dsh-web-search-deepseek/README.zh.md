# dsh-web-search-deepseek

[English](README.md) | 中文

面向 Rust 宿主、由 DeepSeek 支持的 `WebSearchProvider`。它向 `{baseURL}/messages` 发起 `POST`，携带原生 `web_search_20250305`（`name: web_search`，`max_uses` 默认 5），把 `web_search_tool_result` 块映射为 `WebSearchResult`，绝不从模型文本中抓取 URL。YAML 名称为 `@deepseek-ai/dsh-web-search-deepseek`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

提供方复用凭据引用 `DEEPSEEK_API_KEY`（非空的 YAML `apiKey` 优先），通过 `LayeredCredentials` 解析。它**不**读取 `$DEEPSEEK_BASE_URL`。Messages 基址依次为配置 `baseURL`、`$DEEPSEEK_SEARCH_BASE_URL`，否则 `https://api.deepseek.com/anthropic/v1`。HTTP 使用带 `rustls-tls` 与 `redirect::Policy::none()` 的 `reqwest`；3xx 响应是 `WEB_PROVIDER_ERROR`，不会访问 `Location` 目标。请求头为 `x-api-key`、`Authorization: Bearer`、`anthropic-version: 2023-06-01`、JSON 的 `content-type`／`accept`，以及 `user-agent: deepseek-harness/0.0.1`。

`available()` 是本地检查：非空的字面量或已解析密钥、可解析的基址 URL，以及正数 `maxTokens`／`maxUses`。没有结果块时失败为 `WEB_PROVIDER_ERROR`。提供方的 `truncated` 恒为 `false`（由搜索运行时强制 `max_results`）。调用方中止为 `WEB_ABORTED`。本阶段不追加 `web/deepseek-search-llm-request`，也不安装 Settings 段。

## 配置

| 键 | 默认 | 含义 |
|---|---|---|
| `apiKey` | 省略 | 字面量 DeepSeek API 密钥。优先使用 `apiKeyEnv`。 |
| `apiKeyEnv` | `DEEPSEEK_API_KEY` | 每次搜索解析的凭据引用。 |
| `baseURL` | `https://api.deepseek.com/anthropic/v1` | Anthropic 兼容端点基址；会追加 `/messages`。回退到 `$DEEPSEEK_SEARCH_BASE_URL`。 |
| `model` | `deepseek-v4-flash` | Anthropic 格式模型名。 |
| `apiVersion` | `2023-06-01` | `anthropic-version` 请求头值。 |
| `maxTokens` | `4096` | Messages `max_tokens` 正整数。 |
| `maxUses` | `5` | 每次请求原生 `web_search` 使用次数正整数。 |

未知键会在加载时失败。

```yaml
- name: '@deepseek-ai/dsh-web-search-deepseek'
  config:
    apiKeyEnv: DEEPSEEK_API_KEY
```

## 模型体验

独立的 DeepSeek 模型会原样接收 `Perform a web search for the query: <query>` 以及原生 `web_search` 工具。该请求不属于会话模型上下文。通过 [`dsh-tool-web`](../dsh-tool-web/README.md)，会话模型会看到去重后的 URL、标题、日期与引用 snippet；提供方文本不会被信任为 `content`。

#### KV Cache 影响

与会话请求缓存相互独立。

## 已知限制与延后工作

- 一次搜索耗费完整 Messages 模型轮次；DeepSeek 没有专用检索端点。
- 本阶段不把辅助搜索请求写入会话日志。
- 本阶段不把该插件挂入 `base.cordis.yml`。
