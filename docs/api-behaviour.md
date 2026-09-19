# How the TypeSafe API actually behaves

Notes for contributors. TypeSafe's own [documentation](https://docs.typesafe.ai) describes the API;
this page records what the server *does*, measured against the live API, wherever that differs from
what is written or is not written down at all. `jev` is built to the behaviour, not to the prose, so
several of the rules below are the reason a piece of code exists.

Everything here was observed on 2026-09-19 against `jev-1.13.0`.
[`crates/jev-cli/tests/live.rs`](../crates/jev-cli/tests/live.rs) re-checks the load-bearing ones
against the real API every night
([`.github/workflows/live-smoke.yml`](../.github/workflows/live-smoke.yml)). When that job fails,
the server changed: update this page and the validation that depends on the finding.

## The surface

```text
POST https://api.typesafe.ai/v1/systemone
GET  https://api.typesafe.ai/v1/models
Authorization: Bearer $TYPESAFE_API_KEY
```

A request is `state` (string, object or array), an optional `model`, and a map of question id to
question. A response is the resolved `model`, an `answers` map keyed by the same ids, and
`usage.input_tokens` / `usage.output_tokens`. The `x-typesafe-request-id` response header
identifies the call to TypeSafe support, so `jev` surfaces it on success and on every error.

| Type | `criteria` | Answer fields |
| --- | --- | --- |
| `noul` | optional `{"true": …, "false": …}` | `noul` — P(yes), 0–1. **No `confidence`.** |
| `choice` | required map of option to description or `null`, at most 255 options | `choice`, `probabilities`, `confidence` |
| `score` | required ordered array of 2–10 levels; the index is the level | `score` (probability-weighted mean), `legend`, `probabilities`, `confidence` |

Question ids are never shown to the model, so the meaning has to be in `instructions`. `instructions`
and every criteria value accept a string, an object or an array; the field names inside those objects
are labels the model reads, not a reserved vocabulary. State is referenced from an instruction with a
backticked path such as `` `ticket.messages[0].text` ``.

## Where the server differs from its documentation

- **The server does not enforce its own limits.** A Score with a single level is accepted with
  HTTP 200 although the documented minimum is 2. An 11-level Score is refused, but only with
  `400 {"detail": "Too many score levels. Must have at most 10 levels."}`. This is why offline
  validation in `jev-client` is a correctness feature and not a convenience: without it a malformed
  question is either billed and meaningless, or rejected with a message that names no question.
- **An unknown field inside a question is silently accepted.** An unknown *top-level* field, or an
  unknown question `type`, gets the uninformative
  `400 {"detail": {"error_type": "api_usage_error", "message": "Invalid request."}}`; an unknown
  field *inside a question, or inside Noul criteria*, returns 200 and quietly degrades the answer. A
  misspelt `criteria` therefore costs money and produces a plausible wrong number. Client-side
  detection is the only defence, so the request types keep unknown fields in an `extra` map for
  validation to report.
- **A `null` Score level is a 422**, although the documentation lists `null` as an accepted criteria
  value. `null` *is* accepted for a Choice option and for Noul `criteria.true` / `criteria.false`.
- **`instructions` is optional in practice.** A Noul or Choice carrying only `criteria` returns 200.
  A Noul with neither is `400 {"detail": "Noul question must have criteria or instructions: <id>"}`.
- **`state` must be a string, object or array.** `null` reads as "field required" (422), and a bare
  number is a 422.
- **Score `legend` values are not always strings.** The server echoes whatever the question used for
  that level, so an object or array level comes back as an object or array, not the documented
  `map<string, string>`. Over HTTP, `probabilities` and `legend` are keyed by the level as a
  **string** (the Python SDK re-keys them by `int`; the wire format is what `jev-client` parses).
- **Error bodies come in three shapes**, all under `detail`: the documented
  `{"error_type", "message"}` object, a bare string, and — for 422 — a FastAPI list of
  `{"type", "loc", "msg", "input"}`. That list **echoes the request input, including `state`**, so it
  must be treated like a request body when logging and never quoted back to the user verbatim.
- **Field order inside an answer differs from the documentation** (`confidence` precedes
  `probabilities`), and Choice `probabilities` do not come back in the order the options were asked.
  Nothing may depend on either.
- The response `model` is the versioned id (`jev-1.13.0`) even when the request named an alias.
- `GET /v1/models` returns `name`, `description` and an RFC 3339 `release_date` with microseconds,
  and lists only the aliases (`jev-latest`, `jev-preview`). A versioned id that is not listed is
  still accepted by `--model`.

## Model facts and limits

- Price: **$0.042 per million input tokens**; output tokens are free. Cost is always labelled
  *estimated*, because the price table in the binary can go stale.
- Context: 64k tokens for state plus all questions, and 32k for state plus the single longest
  question.
- Rate: roughly 250k tokens/s and 1,200 requests/minute, adjusted dynamically. Concurrency above
  about 8 gets rate-limited on a shared key, which is why batch defaults to 4 and warns above 8.
- Latency is ~100 ms upstream and well under a second end to end from Europe. Every question in a
  request is answered in parallel and independently, so 60 questions in one request cost the same
  wall time as three. Fan-out is cheaper and faster than one call per question.
- Answers are **not bit-for-bit deterministic** (standard deviation ≈ 0.01 on some Nouls). Tests
  assert shapes and ranges, never exact probabilities.
- Retry 408, 429, 5xx (including 529) and connection errors; never retry 400, 401, 403, 404 or 422.
  SDK-parity defaults: 2 retries, 0.5 s initial delay, 5 s maximum, ×2 backoff, 0.25 jitter, 30 s
  timeout, and `retry-after` is honoured.

## Estimating request size

`jev` refuses a request it can see is over budget before spending anything, so it needs a token
estimate it can trust. **A flat characters-per-token ratio is badly wrong.** Measured: English prose
6.5 characters per token, a chat log 4.2, JSON records 2.1, keyed objects 1.3, an array of numbers
1.0 — so the ~4.5 that a first implementation used would estimate a 100k-character JSON state at 22k
tokens when it is about 47k.

What holds across every kind of content, and what
[`crates/jev-client/src/validate/size.rs`](../crates/jev-client/src/validate/size.rs) implements per
character class instead:

- a common word is one token; a rare long word or an identifier is about one token per 4 letters
- CJK is one token per character; accented Latin about one per 3 letters
- **every digit and every punctuation character is one token**
- a line break is one token, an indentation run is one token, a single space is free
- per-request overhead is 258 to 273 tokens (an empty state plus one short Noul is 273)

The estimator is held to within 15% per sample and 5% overall by
[`crates/jev-client/tests/fixtures/token-calibration.json`](../crates/jev-client/tests/fixtures/token-calibration.json),
which stores real token counts from the API. Re-probe and update that file rather than loosening the
bound.

The practical consequence for users is that numeric and id-heavy state is expensive: 3,000
six-digit numbers cost about 20k tokens.

## Weak spots the CLI warns about

Jev reads literally and answers the words, not the intent. It cannot count, do arithmetic or compare
numbers, and it reads dates as text. Those belong in code, and `jev`'s help, MCP tool descriptions
and [agent skill](../skills/jev-cli/SKILL.md) say so wherever an agent will read them.

Two findings are encoded as lint rules in
[`crates/jev-client/src/validate/lints.rs`](../crates/jev-client/src/validate/lints.rs):

- **A Choice without an escape option.** The model cannot pick an option that was not offered, so a
  Choice with no `other` / `none` / `not_stated` forces a wrong answer.
- **Index paths into long arrays get unreliable.** Backticked `items[i]` over 20 unique items was
  20/20 correct; the same items repeated to 60 gave 10 wrong, mostly hovering around 0.51–0.66,
  because locating index 47 is a counting task. For long lists, inline the value in the instruction
  or use a keyed object with short ids, and chunk.

For how to write good questions in general, read TypeSafe's own documentation and agent skill; this
project does not duplicate it.

## Testing against the live API

The key is in `TYPESAFE_API_KEY`. Never `echo` it, never use `set -x` or `curl -v` (which prints the
header), and never paste it into a file, an issue or a pull request.

```bash
curl -sS -X POST https://api.typesafe.ai/v1/systemone \
  -H "Authorization: Bearer $TYPESAFE_API_KEY" \
  -H "Content-Type: application/json" \
  -d @request.json
```
