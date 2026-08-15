# dsh-llm-deepseek

English | [中文](README.zh.md)

DeepSeek `POST /chat/completions` SSE adapter for the Rust host. The adapter serializes harness messages (assistant `content` is `""`, never JSON `null`), parses SSE until `[DONE]`, translates wire chunks, and applies a per-read idle watchdog.

The bearer token is resolved on every `stream` call from the connection snapshot for that call. The adapter struct does not store the key. HTTP uses `reqwest` with `rustls-tls` (no OpenSSL / `native-tls`).

## Known Limitations and Deferred Work

- Model catalog, retry policy, and quota/context-window classifiers beyond the status map are later work. Phase 3 exit is serialize / SSE / translate / header / per-request key / idle timeout against an in-process mock server.
