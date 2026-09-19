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

## Wire-shape findings (2026-09-19, while building the `jev-client` types)

Where these contradict the upstream docs, the server's behaviour is what the code follows.

- **Score `legend` values are not always strings.** The server echoes whatever the question used for
  that level, so an object or array level comes back as an object or array. (Docs: `map<string, string>`.)
- **A `null` Score level is rejected with 422**, although the docs list `null` as accepted for score
  levels. `null` is accepted for a Choice option and for Noul `criteria.true` / `criteria.false`.
- **`state` must be a string, object or array.** `null` reads as "field required" (422); a number is a 422.
- **`instructions` is optional in practice.** A Noul or Choice with only `criteria` returns 200. A Noul
  with neither returns `400 {"detail": "Noul question must have criteria or instructions: <id>"}`.
- **Unknown fields are handled inconsistently.** An unknown *top-level* field, or an unknown question
  `type`, gets an uninformative `400 {"detail": {"error_type": "api_usage_error", "message": "Invalid request."}}`.
  An unknown field *inside a question or inside Noul criteria* is **silently accepted** (200), so a
  misspelt `criteria` quietly degrades answers. Client-side detection is the only defence.
- **Error bodies come in three shapes**, all under `detail`: the documented object
  `{"error_type", "message"}`; a bare string; and, for 422, a FastAPI list of
  `{"type", "loc", "msg", "input"}`. Note that the 422 list **echoes the request input**, including
  `state`, so it must be treated like a request body when logging.
- The response `model` is the versioned id (`jev-1.13.0`) even when the request used an alias.
- Field order inside an answer differs from the docs (`confidence` precedes `probabilities`), and Choice
  `probabilities` do not come back in the order the options were asked.
- `GET /v1/models` returns `name`, `description` and an RFC 3339 `release_date` with microseconds.

## Token-size findings (2026-09-19, while building offline validation)

- **A flat characters-per-token ratio is badly wrong.** Measured: English prose 6.5 chars/token, a chat
  log 4.2, JSON records 2.1, keyed objects 1.3, an array of numbers 1.0. The ~4.5 in the PRD would
  estimate a 100k-character JSON state at 22k tokens when it is about 47k.
- What holds across every kind of content: a common word is one token, a rare long word or an
  identifier about one token per 4 letters, CJK one token per character, accented Latin about one per
  3 letters, **every digit and every punctuation character one token**, a line break one token, an
  indentation run one token, a single space free.
- Per-request overhead: 258 to 273 tokens (an empty state plus one short Noul is 273).
- `jev-client`'s estimator implements this and is held to within 15% per sample (5% overall) by
  `crates/jev-client/tests/fixtures/token-calibration.json`, which stores real counts.
- Practical consequence for users: numeric and id-heavy state is expensive. 3,000 six-digit numbers
  cost 20k tokens.
