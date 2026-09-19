# Documentation

## Using `jev`

| Page | Covers |
| --- | --- |
| [commands.md](commands.md) | Every command, flag, default and example — generated from the command tree, so it matches the binary. Regenerate it with `make reference`. |
| [exit-codes.md](exit-codes.md) | The exit-code contract, the gating flags, and the JSON that `jev` prints on stdout and stderr. |
| [configuration.md](configuration.md) | Settings and their precedence, the configuration file, profiles, the API key, environment variables and updates. |
| [threat-model.md](threat-model.md) | The long form of [SECURITY.md](../SECURITY.md): what is protected, which inputs are not trusted, where each protection lives and how it is tested, and the findings of the last review. Update it with any change that moves a trust boundary. |

The [README](../README.md) is the place to start, [SECURITY.md](../SECURITY.md) has the security
model, and [`skills/jev-cli/SKILL.md`](../skills/jev-cli/SKILL.md) is the same ground written for
an AI agent.

# Development context

Background material for building `jev-cli`. The rest of this directory is not user-facing
documentation for the CLI itself.

## `prd/` — product requirements (committed)

[prd/v1.md](prd/v1.md) is the PRD for the first version of the CLI: scope, numbered requirements, exit-code contract, security model, milestones, and the log of decisions behind them.

## `context/` — study notes (committed)

Written in our own words from the TypeSafe docs plus live API calls. Read in order:

| File | Covers |
| --- | --- |
| [01-overview.md](context/01-overview.md) | What Jev is and is not |
| [02-http-api.md](context/02-http-api.md) | Endpoints, request/response shapes, errors, pricing, limits, SDKs, keeping the key out of logs |
| [03-design-rules.md](context/03-design-rules.md) | How to write good questions; documented weak spots of jev-1.13 |
| [04-cookbook-techniques.md](context/04-cookbook-techniques.md) | Reusable techniques and reported numbers from all 18 upstream cookbooks |
| [05-live-test-findings.md](context/05-live-test-findings.md) | Behaviour observed against the real API that the docs don't mention |

Facts a CLI has to respect, with where they come from:

- One endpoint does the work: `POST /v1/systemone`; `GET /v1/models` lists aliases. Auth is a bearer token from `TYPESAFE_API_KEY`; `TYPESAFE_BASE_URL` and `TYPESAFE_DEFAULT_MODEL` are the SDKs' other env vars. (02)
- Limits: 255 options per Choice, 2–10 levels per Score, 64k tokens per request, 32k for state plus the longest question. The server does not enforce all of them cleanly — a 1-level Score returns 200 and an 11-level Score gets only a bare 400 — so validate before sending. (02, 04, 05)
- Retry 408/429/5xx (including 529) with exponential backoff and honour `retry-after`. Concurrency above roughly eight gets rate-limited. (02, 04)
- Responses carry the resolved model id (`jev-1.13.0`) and an `x-typesafe-request-id` header; both are worth surfacing. Aliases move without notice. (02)
- Only input tokens are billed ($0.042 per million), so cost is computable from `usage.input_tokens`. (02)
- Score `probabilities` and `legend` are keyed by string level over HTTP. (02)
- Answers are not bit-for-bit deterministic (std ≈ 0.01 on some Nouls). (04)

## `upstream/` — mirror of docs.typesafe.ai (gitignored)

A verbatim Markdown copy of every page in the upstream index, for grepping and for feeding to coding agents. It is TypeSafe's content and carries no open licence, so it stays out of this public repository. Recreate or refresh it with:

```bash
scripts/fetch-upstream-docs.sh
```

Useful entry points once mirrored: `upstream/llms.txt` (index), `upstream/api.md`, `upstream/models.md`, `upstream/primitives/`, `upstream/model-jaggedness/jev-1.13.md`, `upstream/cookbooks/`, `upstream/sdk/python/` and `upstream/sdk/javascript/` (reference implementations of retries, errors and typing).
