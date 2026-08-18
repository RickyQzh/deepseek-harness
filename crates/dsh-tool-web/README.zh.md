# dsh-tool-web

[English](README.md) | 中文

面向模型的 `web_search`，构建于 [`dsh-web`](../dsh-web/README.md) 之上。本 crate 负责工具名称、JSON schema、参数检查、`searchMaxResults` 上限、markdown 格式，以及呈现意图（`generic`／kind `search`）。它不选择提供方，也不发起 HTTP。YAML 名称为 `@deepseek-ai/dsh-tool-web`。本 crate 不加入 `register_spine_plugins`，也不挂入 `base.cordis.yml`。

`web_fetch` 保持关闭。配置 `fetch` 默认是 **false**。`fetch: true` 会在注入 `tools` 或 `web` 之前以 `web_fetch remains off in Phase 6` 使插件加载失败。不注册该工具，提示词也不推荐 `web_fetch`。`is_concurrency_safe` 为 `true`。空或仅空白的 `query` 失败为 `query must be a non-empty string`（工具错误字符串，不是 `WebError`）。

`format_search_output` 与 TypeScript markdown 一致：标题或主机名、snippet、日期、`No results found.`、截断说明，以及引用来源的指令。`search_meta_from_result` 构造 `WebSearchMeta { sources, truncated, answer? }`。`ToolRuntime::dispatch` 目前将 `meta` 设为 `None`；该辅助函数尚未接入 `dsh-tools`。

## 配置

| 键 | 默认 | 含义 |
|---|---|---|
| `search` | `true` | 注册 `web_search`。 |
| `fetch` | `false` | 必须保持 false。`true` 会使加载失败。 |
| `searchMaxResults` | `8` | 一次 `web_search` 调用返回的来源数量上限。 |
| `searchTimeoutMs` | `60000` | 协作式工具调用超时预算（ms）。本阶段不附加到 `ToolDefinition`。 |

未知键会在加载时失败。出现上限键时必须是正整数。

```yaml
- name: '@deepseek-ai/dsh-tool-web'
  config:
    search: true
    fetch: false
    searchMaxResults: 8
    searchTimeoutMs: 60000
```

## 模型体验

模型看到工具 `web_search`，参数为 `query`。结果文本是格式化后的来源列表（或 `No results found.`），外加 `Cite the relevant URLs above as markdown links in your answer.`

#### KV Cache 影响

可复用请求前缀之后，工具结果只追加。

## 已知限制与延后工作

- 本阶段因 SSRF 省略 `web_fetch`，留待后续阶段。
- 在 `dsh-tools` 为 dispatch 增加 meta 槽位之前，呈现元数据不会附到 `ToolExecutionResult` 上。
- 本阶段不把该插件挂入 `base.cordis.yml`。
