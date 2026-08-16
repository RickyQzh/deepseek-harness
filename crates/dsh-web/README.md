# dsh-web

English | [中文](README.zh.md)

Web search Service Definition for the DeepSeek Harness Rust host. `plugin::register` provides `web` as a `WebRuntime`. `inject::<WebRuntime>()` yields `Arc<WebRuntime>`, so `register_search_provider` and `search` take `&self` with an interior mutex. YAML name `@deepseek-ai/dsh-web`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

Fetch types, `register_fetch_provider`, and `web_fetch` are omitted in this phase because URL retrieval is SSRF-sensitive. Search is the only operation.

Selection is resolved at execution time and never depends on registration order. A configured id that is missing fails as `WEB_PROVIDER_CONFIGURED_MISSING` (`configured web provider "{id}" is not registered`). A configured id that is registered but `available() == false` fails as `WEB_PROVIDER_CONFIGURED_UNAVAILABLE`. With no configured id, exactly one usable provider is selected; several usable providers fail as `WEB_PROVIDER_AMBIGUOUS`; none fail as `WEB_PROVIDER_UNAVAILABLE`. Duplicate ids fail as `WEB_DUPLICATE_PROVIDER` (`a web provider with id "{id}" is already registered`). `search` caps `sources` to `max_results` and sets `truncated` when the provider over-returns; a provider `truncated` flag is kept when the list is already within the cap.

`AbortFlag::is_aborted()` fails as `WEB_ABORTED`. `WebError` Display is the message; `code` is the stable routing token.

## Config

| Key | Default | Meaning |
|---|---|---|
| `searchProvider` | omitted | Pin the search provider id. Omitted = auto-select when exactly one usable provider is registered. |

Unknown keys fail load.

## Model Experience

Indirectly through [`dsh-tool-web`](../dsh-tool-web/README.md). This registry contributes no tool schema or prompt.

#### KV Cache effect

No direct invalidation.

## Known Limitations and Deferred Work

- Fetch (SSRF-sensitive URL retrieval) is omitted until a later phase.
- Provider unregistration / HMR disposers are not implemented.
- The plugin is not mounted in `base.cordis.yml` in this phase.
