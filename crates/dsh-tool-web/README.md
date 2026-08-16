# dsh-tool-web

English | [中文](README.zh.md)

Model-facing `web_search` over [`dsh-web`](../dsh-web/README.md). This crate owns the tool name, JSON schema, argument checks, `searchMaxResults` bound, markdown formatting, and presentation intent (`generic` / kind `search`). It does not select providers or perform HTTP. YAML name `@deepseek-ai/dsh-tool-web`. This crate is not added to `register_spine_plugins` and is not mounted in `base.cordis.yml`.

`web_fetch` stays off. Config `fetch` defaults to **false**. `fetch: true` fails plugin load with `web_fetch remains off in Phase 6` before `tools` or `web` are injected. The tool is not registered, and no prompt text recommends `web_fetch`. `is_concurrency_safe` is `true`. Empty or whitespace `query` fails with `query must be a non-empty string` (a tool error string, not a `WebError`).

`format_search_output` matches the TypeScript markdown: title or hostname, snippets, dates, `No results found.`, a truncation note, and the cite-your-sources instruction. `search_meta_from_result` builds `WebSearchMeta { sources, truncated, answer? }`. `ToolRuntime::dispatch` currently sets `meta: None`; this helper is not wired into `dsh-tools`.

## Config

| Key | Default | Meaning |
|---|---|---|
| `search` | `true` | Register `web_search`. |
| `fetch` | `false` | Must stay false. `true` fails load. |
| `searchMaxResults` | `8` | Upper bound on sources returned by one `web_search` call. |
| `searchTimeoutMs` | `60000` | Cooperative tool-call timeout budget (ms). Not attached to `ToolDefinition` in this phase. |

Unknown keys fail load. Positive integer caps are required when those keys are present.

```yaml
- name: '@deepseek-ai/dsh-tool-web'
  config:
    search: true
    fetch: false
    searchMaxResults: 8
    searchTimeoutMs: 60000
```

## Model Experience

The model sees tool `web_search` with argument `query`. Result text is the formatted source list (or `No results found.`) plus `Cite the relevant URLs above as markdown links in your answer.`

#### KV Cache effect

Append-only tool results after the reusable request prefix.

## Known Limitations and Deferred Work

- `web_fetch` is omitted (SSRF) until a later phase.
- Presentation meta is not attached to `ToolExecutionResult` until `dsh-tools` grows a meta slot on dispatch.
- The plugin is not mounted in `base.cordis.yml` in this phase.
