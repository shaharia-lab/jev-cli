# Live test findings (2026-09-19)

- `GET /v1/models` lists `jev-latest`, `jev-preview`.
- 3-question request: 465 input tokens, ~0.7 s wall time from Europe, 103 ms upstream (`x-envoy-upstream-service-time`).
- Per-request overhead is ~270 input tokens even for a one-word state.
- 60 Nouls in one request: same 0.7 s wall time.
- **A Score with a single level is accepted (HTTP 200)** despite the docs saying ≥2 → don't rely on
  the server for validation; validate question shapes client-side.
- **Index paths into long arrays get unreliable.** `items[i]` over 20 unique items: 20/20 correct.
  Same items repeated to 60: 10 wrong, mostly hovering 0.51–0.66. Locating index 47 is a counting
  task. For long lists, inline the value in the instruction or use a keyed object with short ids,
  and chunk.
