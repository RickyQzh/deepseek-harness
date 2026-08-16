# dsh-llm

English | [中文](README.zh.md)

Provider-neutral LLM stream contract for the Rust host. Adapters emit `usage` before the terminal `finish` and nothing afterward. `LlmRuntime::stream` converts adapter failures into that terminal `finish` (`error` or `aborted`); consumers do not catch thrown adapter errors. `LlmRuntime` is `Clone` (the adapter map) and `list_providers` returns registered route ids.

`StreamChunk`, `FinishReason`, `TokenUsage`, and `Message` are the Phase 2 `dsh-session` types. `ToolSchema` is declared here because it rides on `GenerateOptions`. `BlockAssembler` is the loop's chunk-to-message fold, including the max-tokens rule that drops tool-call blocks.

`plugin::register_llm` provides `llm`. `plugin::register_mock` registers `MockAdapter` on config `provider` (default `mock`). `plugin::register_retry` mounts YAML `@deepseek-ai/dsh-llm-retry` and registers the `agent/request-error` waterfall that executes `LlmAdapter::retry_policy` when a `RetryScope` is active. Default retryable codes are `EMPTY_RESPONSE` and `RATE_LIMIT` only, so `CONTEXT_WINDOW_EXCEEDED` (`CONTEXT_WINDOW_EXCEEDED_CODE`) delegates and compaction can recover. `replay::register` serves `assistant/chunk` runs from `DSH_SNAPSHOT_FILE` on each config `providers[].id` and returns `LlmModelContext { context_window }` from YAML `providers[].models[].contextWindow`. `retry_snapshot::register_retry_snapshot_backend` mounts YAML `retry-snapshot-backend` on `deepseek-official` (first stream `RATE_LIMIT` at HTTP 429, then text `RETRY_OK` with unchanged messages).

## Known Limitations and Deferred Work

- Adapter registry replace/dispose, configurable-provider directory, and model discovery are later phases. Phase 3 is a `HashMap` of adapters plus `prepare_call` default materialization.
