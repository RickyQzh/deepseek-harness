# dsh-web-search-deepseek

English | [中文](README.zh.md)

DeepSeek-backed `WebSearchProvider` for the Rust host. It `POST`s `{baseURL}/messages` with native `web_search_20250305` (`name: web_search`, `max_uses` default 5), maps `web_search_tool_result` blocks into `WebSearchResult`, and never scrapes URLs out of model prose. YAML name `@deepseek-ai/dsh-web-search-deepseek`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

The provider reuses credential reference `DEEPSEEK_API_KEY` (optional YAML `apiKey` wins when non-empty) through `LayeredCredentials`. It does **not** read `$DEEPSEEK_BASE_URL`. The Messages base is config `baseURL`, else `$DEEPSEEK_SEARCH_BASE_URL`, else `https://api.deepseek.com/anthropic/v1`. HTTP uses `reqwest` with `rustls-tls` and `redirect::Policy::none()`; a 3xx response is `WEB_PROVIDER_ERROR` and the `Location` target is not contacted. Headers are `x-api-key`, `Authorization: Bearer`, `anthropic-version: 2023-06-01`, JSON `content-type`/`accept`, and `user-agent: deepseek-harness/0.0.1`.

`available()` is local: a non-empty literal or resolved key, a parseable base URL, and positive `maxTokens`/`maxUses`. No result blocks, or a present non-array `web_search_tool_result.content`, fail as `WEB_PROVIDER_ERROR`. Missing or JSON-null `content` is an empty item list. Provider `truncated` is always `false` (the search runtime enforces `max_results`). Caller abort is `WEB_ABORTED`. This phase does not append `web/deepseek-search-llm-request` and does not install a Settings section.

## Config

| Key | Default | Meaning |
|---|---|---|
| `apiKey` | omitted | Literal DeepSeek API key. Prefer `apiKeyEnv`. |
| `apiKeyEnv` | `DEEPSEEK_API_KEY` | Credential reference resolved for each search. |
| `baseURL` | `https://api.deepseek.com/anthropic/v1` | Anthropic-compatible endpoint base; `/messages` is appended. Falls back to `$DEEPSEEK_SEARCH_BASE_URL`. |
| `model` | `deepseek-v4-flash` | Anthropic-format model name. |
| `apiVersion` | `2023-06-01` | `anthropic-version` header value. |
| `maxTokens` | `4096` | Positive-integer Messages `max_tokens`. |
| `maxUses` | `5` | Positive-integer native `web_search` uses per request. |

Unknown keys fail load.

```yaml
- name: '@deepseek-ai/dsh-web-search-deepseek'
  config:
    apiKeyEnv: DEEPSEEK_API_KEY
```

## Model Experience

A separate DeepSeek model receives exactly `Perform a web search for the query: <query>` plus the native `web_search` tool. That request is not part of the conversation model's context. Through [`dsh-tool-web`](../dsh-tool-web/README.md), the conversation model sees deduplicated URLs, titles, dates, and citation snippets; provider prose is not trusted as `content`.

#### KV Cache effect

Independent of the conversation request cache.

## Known Limitations and Deferred Work

- One search costs a full Messages model turn; DeepSeek exposes no dedicated retrieval endpoint.
- Auxiliary search requests are not written to the session log in this phase.
- The plugin is not mounted in `base.cordis.yml` in this phase.
