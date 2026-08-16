# dsh-token-meter

English | [中文](README.zh.md)

Replay-aware token measurement for the Rust host. `plugin::register` provides `tokenMeter` as a single `TokenMeter` value (not `Arc<TokenMeter>`). YAML config must be omitted or an empty object; any key fails load with `unknown key`.

`TokenMeter::measure` catches one session's fold up through the durable log, keyed by `SessionId` string, and returns a detached snapshot. `total_tokens` is request-and-response pressure: `max(0, baseline + surface_delta)`. `surface_tokens` is the surface-only heuristic total and equals the sum of `nodes[].tokens`. A `request_header` override changes pressure fields only; surface fields still describe the current session.

Estimation uses a fixed four-characters-per-token heuristic plus structural overhead (`ROLE_OVERHEAD` on messages and system text, `BLOCK_OVERHEAD` on content blocks and tool JSON). Provider usage is reused only when the measured canonical header equals the latest successful-call anchor and that usage is at least the matching full heuristic price; otherwise the envelope and surface are estimated. Usage sums disjoint input, cache-read, cache-write, and output; reasoning is not added again. An explicit empty `sourceEventSeqs` list prices a known empty provider stream; an absent list conservatively treats durable assistant output as provider output.

This crate does not register session-projection units and is not added to `register_spine_plugins`.

## Known Limitations and Deferred Work

- The fixed heuristic is approximate: content without reusable provider usage is priced by character count plus structural overhead, not an exact provider tokenizer or request serializer.
- Every measurement clones the current surface, so reads are O(surface).
- Provider usage is only reusable for an identical canonical envelope.
- Missing legacy source seqs are handled conservatively: assistant messages without `sourceEventSeqs` cannot distinguish provider output from listener rewrites.
