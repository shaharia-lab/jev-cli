# HTTP API

```
POST https://api.typesafe.ai/v1/systemone
GET  https://api.typesafe.ai/v1/models
Authorization: Bearer $TYPESAFE_API_KEY
Content-Type: application/json
```

Request: `{ "state": string|object|array, "model": "jev-latest", "questions": { "<id>": Question } }`

| Type | `criteria` | Answer |
| --- | --- | --- |
| `noul` | optional `{ "true": ..., "false": ... }` | `noul` (P(yes), 0–1). **No confidence field.** |
| `choice` | required map `option -> description \| null`, up to 255 options | `choice`, `probabilities{}`, `confidence` |
| `score` | required ordered array, 2–10 levels, index = level | `score` (prob-weighted mean), `legend{}`, `probabilities{}`, `confidence` |

- `instructions` and every criteria value accept string, object, array, or null. Field names inside
  those objects (`question`, `focus`, `what`, `not_for`, `examples`, `signals`, `compare`, `inspect`)
  are *not* reserved — the model sees them as labels.
- Question ids are never sent to the model. Put full meaning in `instructions`.
- Reference state with backticked paths: `` `ticket.messages[0].text` ``.
- Response: `model` (the resolved versioned id, e.g. `jev-1.13.0`), `answers{}`, `usage{input_tokens, output_tokens}`.
- Response header `x-typesafe-request-id` — log it.
- Errors: 401, 422 (validation), 429 (rate limit), 529 (overloaded). Body is
  `{"detail": {"error_type": "...", "message": "..."}}`. Retry 429/529 with exponential backoff and
  honor `retry-after`. SDK defaults: 2 retries, 0.5 s initial, 5 s max, 0.25 jitter, 30 s timeout,
  retries 408/429/5xx.

### Model facts (jev-1.13.0, as of 2026-09-17)

- Price: **$0.042 per million input tokens**; output tokens free.
- Limits: 250k tokens/s, 1,200 req/min — "adjusting dynamically".
- Context: 64k tokens for state + all questions; 32k for state + the single longest question.
- Aliases `jev-latest` and `jev-preview` both resolve to `jev-1.13.0`. Aliases move silently —
  **pin the versioned id once thresholds are tuned.**
- ~100 ms typical; all questions in a request run in parallel and independently.

### SDKs

- Python ≥3.10: `pip install typesafe-sdk` → `TypeSafeClient` / `AsyncTypeSafeClient`,
  `client.system_one(state, questions, response_model=...)`, `Choice`/`Score`/`Noul`/`NoulCriteria`.
  Result has `.answers`, `.nouls`, `.choices`, `.scores`, `.request_id`, `.raw_http_response`.
  Python keys score `probabilities`/`legend` by **int**, HTTP by **string**.
- JS/TS (Node ≥20): `npm install @typesafe-ai/sdk` → `client.systemOne({state, questions})`,
  helpers `choice()`, `score()`, `noul()`; answer types inferred from questions.
- Env: `TYPESAFE_API_KEY`, `TYPESAFE_BASE_URL`, `TYPESAFE_DEFAULT_MODEL`, `TYPESAFE_LOG_LEVEL`.
  `debug` logging redacts auth headers but **not bodies**.
- Old preview API (`/preview/evaluation`, `prompts`/`document`/`chosen`/`expectation`) is dead; ignore
  any example using those names.


## Calling it without leaking the key

```bash
curl -sS -X POST https://api.typesafe.ai/v1/systemone \
  -H "Authorization: Bearer $TYPESAFE_API_KEY" -H "Content-Type: application/json" -d @request.json
```

Never `echo` the variable, never `set -x`, never `curl -v` (prints the header).
