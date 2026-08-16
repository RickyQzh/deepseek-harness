# dsh-llm

English | [中文](README.zh.md)

Provider-neutral LLM stream contract for the Rust host. Adapters emit `usage` before the terminal `finish` and nothing afterward. `LlmRuntime::stream` converts adapter failures into that terminal `finish` (`error` or `aborted`); consumers do not catch thrown adapter errors. `LlmRuntime` is `Clone` (the adapter map) and `list_providers` returns registered route ids.

`StreamChunk`, `FinishReason`, `TokenUsage`, and `Message` are the Phase 2 `dsh-session` types. `ToolSchema` is declared here because it rides on `GenerateOptions`. `BlockAssembler` is the loop's chunk-to-message fold, including the max-tokens rule that drops tool-call blocks.

## Known Limitations and Deferred Work

- Adapter registry replace/dispose, configurable-provider directory, model discovery, and `dsh-llm-retry` are later phases. Phase 3 is a `HashMap` of adapters plus `prepare_call` default materialization.
