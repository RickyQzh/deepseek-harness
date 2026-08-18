# dsh-token-meter

[English](README.md) | 中文

面向 Rust 宿主的重放感知 token 计量。`plugin::register` 以单个 `TokenMeter` 值提供 `tokenMeter`（不是 `Arc<TokenMeter>`）。YAML 配置必须省略或为空对象；任何键都会在加载时失败，错误信息包含 `unknown key`。

`TokenMeter::measure` 按 `SessionId` 字符串键值把一个会话的 fold 追到持久化日志末尾，并返回一份分离的快照。`total_tokens` 是请求与响应压力：`max(0, baseline + surface_delta)`。`surface_tokens` 是仅 surface 的启发式合计，等于 `nodes[].tokens` 之和。`request_header` 覆盖只改变压力字段；surface 字段仍描述当前会话。

估算使用固定的每四字符一 token 启发式，外加结构开销（消息与系统文本用 `ROLE_OVERHEAD`，内容块与工具 JSON 用 `BLOCK_OVERHEAD`）。仅当被测的规范 header 等于最近一次成功调用的锚点、且该 usage 不低于对应的完整启发式价格时，才复用提供方 usage；否则估算整个信封与 surface。Usage 合计互不相交的 input、cache-read、cache-write 与 output；不会把 reasoning 再加一次。显式空的 `sourceEventSeqs` 列表按已知空提供方流计价；缺省列表则保守地把持久化 assistant 输出当作提供方输出。

本 crate 不注册 session-projection 单元，也不加入 `register_spine_plugins`。

## 已知限制与暂缓事项

- 固定启发式是近似值：没有可复用提供方 usage 的内容按字符数加结构开销计价，不是精确的提供方 tokenizer 或请求序列化器。
- 每次计量都会克隆当前 surface，因此读取是 O(surface)。
- 提供方 usage 仅在规范信封完全相同时可复用。
- 缺失的遗留 source seqs 按保守方式处理：没有 `sourceEventSeqs` 的 assistant 消息无法区分提供方输出与监听器改写。
