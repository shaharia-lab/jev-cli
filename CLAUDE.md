# jev-cli — guide for AI agents and contributors

`AGENTS.md` is a symlink to this file. Edit `CLAUDE.md` only.

## What this project is

`jev` is an **unofficial**, open-source Rust CLI for TypeSafe AI's **Jev** model (not affiliated with
TypeSafe AI). Jev is a "System One" model: it never generates text. You send a `state` plus a map of
typed questions (`noul` = yes/no probability, `choice` = one of up to 255 options, `score` = position on
a 2–10 level rubric) and get calibrated probabilities back. The CLI makes that usable from a terminal,
shell scripts and CI (answers become exit codes), bulk jobs, and AI agents (including an MCP server mode).

Status: **early implementation.** The workspace, CI and governance files exist; the command surface
does not yet (the `jev` binary only prints its version). Work is tracked in epic
[#2](https://github.com/shaharia-lab/jev-cli/issues/2) with sub-issues linked by native blocked-by
relationships. Pick issues whose blockers are closed.

## Read before designing or coding

| Need | Read |
| --- | --- |
| Scope, numbered requirements (`FR-`, `NFR-`, `SEC-`, `REL-`, `ENG-`, `TEST-`, `DIST-`), exit codes, decision log | `docs/prd/v1.md` |
| API shapes, errors, limits, pricing, env vars | `docs/context/02-http-api.md` |
| Server quirks found in live testing | `docs/context/05-live-test-findings.md` |
| How good questions are written; model weak spots | `docs/context/03-design-rules.md` |
| Full upstream docs (gitignored mirror) | run `scripts/fetch-upstream-docs.sh`, then `docs/upstream/` |

The planning docs under `docs/prd/` and `docs/context/` will be removed before the first public release.
Anything durable belongs in this file, the README, or code comments.

Decisions in the PRD's decision log (§16) are settled. Do not re-open them; raise a question instead.

## Non-negotiable product rules

1. **Humans and AI agents are equal first-class users.** Every command's help must state its purpose,
   *when to use it versus sibling commands*, inputs, output contract, exit codes, and 2–4 copy-pasteable
   examples. An agent reading only the help text must be able to call the command correctly.
2. **stdout is data; stderr is everything else** (progress, notices, warnings, logs, errors).
3. **Never prompt** when stdin is not a TTY, `--no-input` / `JEV_NO_INPUT` is set, or `CI=true`. Fail with
   an error that names the flag or env var to use.
4. **Validate before spending.** The server does not enforce limits reliably (a 1-level Score returns 200,
   an 11-level Score returns a server error), so client-side validation is a correctness feature.
5. **Exit codes are a stable contract:** 0 ok · 1 internal · 2 usage/validation · 3 auth · 4 API rejected ·
   5 rate-limited/overloaded · 6 network/timeout · 7 batch partial failure · **10 gate condition false** ·
   11 abstain band · 20 update available · 130 interrupted. Exit 10 is never used for errors.
6. Flags, JSON output shapes and exit codes are versioned API (SemVer). Changing them is a breaking change.

## Security rules

- The API key is **never** accepted as a CLI argument value and never appears in config files, logs,
  error messages, `--dry-run` output or MCP results. Hold it in a zeroize-on-drop type with a redacting `Debug`.
- Never log request or response bodies unless `--debug-bodies` is explicitly passed; `state` often
  contains customer data. `Authorization` is always redacted.
- `https://` only, except loopback. TLS through `rustls`; certificate verification cannot be disabled.
- No telemetry. The only hosts contacted are the configured TypeSafe base URL and GitHub Releases for
  this repo (updates).
- Self-update verifies signature **and** SHA-256 against a public key compiled into the binary, and
  never downgrades automatically.
- MCP file access is deny-by-default and confined to `--allow-dir` roots.
- `#![forbid(unsafe_code)]` in both crates.
- When testing against the live API: the key is in `TYPESAFE_API_KEY`. Never `echo` it, never use
  `set -x` or `curl -v`, never paste it into files, issues or PRs.

## Architecture

Cargo workspace, one-way dependency `jev-cli → jev-client`:

- `crates/jev-client` — library: typed requests/answers (`serde` + `schemars`), offline validation,
  HTTP transport (`reqwest` + `rustls`), SDK-parity retries, typed errors, pricing. **No** clap, terminal,
  or config-file concerns. Usable as a standalone Rust client.
- `crates/jev-cli` — binary `jev`: `clap` command tree (single source for `--help` and `jev spec`),
  one module per command group under `commands/`, config and profiles, credential stores, output
  renderers, batch engine, MCP stdio server, updater.

The `jev-client` types follow three rules. Requests are sent as written (key order, explicit `null`
and unknown fields are all preserved). Responses parse tolerantly (unknown fields ignored; an unknown or
reshaped answer becomes `Answer::Unknown` with its raw JSON). Every type derives `JsonSchema`, and a
type whose doc comment has rustdoc links or examples sets `#[schemars(description = "...")]`, because
agents read those descriptions (a test enforces this). Public structs are `#[non_exhaustive]` with
constructors, so adding a field is not a breaking change.

Commands depend on traits (`Transport`, `CredentialStore`, `Clock`, `UpdateSource`) so they are testable
with fakes. Errors use `thiserror` in the library; the binary has one error → exit-code mapping.

## API facts that code must respect

- `POST /v1/systemone` does the work; `GET /v1/models` lists aliases. Bearer auth.
- Env vars shared with the official SDKs: `TYPESAFE_API_KEY`, `TYPESAFE_BASE_URL`,
  `TYPESAFE_DEFAULT_MODEL`, `TYPESAFE_LOG_LEVEL`. CLI-only settings use the `JEV_` prefix.
- Retry 408/429/5xx (incl. 529) and connection errors: 2 retries, 0.5 s initial, 5 s max, 0.25 jitter,
  30 s timeout, honour `retry-after`. Never retry 400/401/403/404/422.
- Concurrency above roughly 8 gets rate-limited on shared keys; batch defaults to 4.
- Surface the resolved model id (`jev-1.13.0`) and the `x-typesafe-request-id` header. Aliases
  (`jev-latest`) move without notice.
- Only input tokens are billed ($0.042 per million for `jev-1.13.0`); cost is always labelled *estimated*.
- Score `probabilities` and `legend` are keyed by **string** level over HTTP. `legend` values echo the
  question's levels, so they can be objects or arrays, not only strings.
- The upstream docs are not the last word; live behaviour is (`docs/context/05-live-test-findings.md`).
  A `null` Score level is a 422 although documented as allowed; `instructions` is optional in practice.
- An unknown top-level request field gets an opaque `400 Invalid request.`, but an unknown field
  *inside a question* is silently accepted, so a misspelt `criteria` quietly degrades answers. The
  request types keep unknown fields (`extra`) so validation can report them.
- Error bodies have three shapes under `detail`: the documented `{error_type, message}` object, a bare
  string, and a FastAPI list for 422 that **echoes the request input** (treat it like a body when logging).
- Answers are not bit-for-bit deterministic. Tests assert shapes and ranges, never exact probabilities.
- Parse responses tolerantly: unknown fields and unknown answer types must not crash the CLI.

## Workflow

- Conventional Commits for PR titles (`feat:`, `fix:`, `docs:`, `chore:` …); release-please cuts releases.
- `main` requires a pull request, signed commits and linear history. Work on a branch, one issue per PR,
  and reference the issue and the PRD requirement ids it satisfies.
- Run every CI gate locally with **`make check`** before pushing. Individually:

  | Gate | Command |
  | --- | --- |
  | Format | `cargo fmt --all --check` (fix: `cargo fmt --all`) |
  | Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
  | Test | `cargo test --workspace --all-features --locked` |
  | MSRV | `make msrv` (checks with `rust-version` from `Cargo.toml`) |
  | Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked` |
  | Supply chain | `cargo deny --locked check` and `cargo audit --deny unsound --deny yanked` |
  | Repo policy | `scripts/ci/check-action-pins.sh`, `scripts/ci/check-client-deps.sh` |

- Git hooks are defined in `.pre-commit-config.yaml`; install them once per clone with
  `pre-commit install` (commit: hygiene, fmt, clippy, no commits to `main`; push: tests and the crate
  boundary). Never bypass them with `--no-verify`: fix what they report. A worktree shares the main
  checkout's hooks, so they run there too.
- If `cargo` is not on `PATH` in a fresh shell, run `. "$HOME/.cargo/env"` first.
- The development toolchain is pinned in `rust-toolchain.toml` (Dependabot bumps it). The MSRV is
  `rust-version` in the root `Cargo.toml`; policy is latest stable minus two, changed deliberately.
- Lints live in `[workspace.lints]` in the root `Cargo.toml` and `clippy.toml`. `unwrap`, `expect`,
  `panic!` and slice indexing are linted outside tests: return an error instead. Fix the cause
  rather than adding `#[allow]`; an unavoidable allow needs a comment saying why.
- Dependency policy is `deny.toml`: permissive licences only, no OpenSSL or `native-tls`, crates.io
  only. `jev-client` must not depend on CLI, terminal or config crates (CI checks this).
- Every third-party GitHub Action is pinned to a full commit SHA with the version in a comment.
- Tests run against a local mock server (`wiremock`); CLI behaviour is asserted with `assert_cmd` and
  snapshot tests on stdout, stderr **and** exit code. Live API tests are opt-in only.
- Isolate all test state with `JEV_CONFIG_DIR`; never touch the real user config or keychain.
- Match the surrounding code's naming, idiom and comment density. Document public items in `jev-client`.

## Distribution

GitHub Releases + install scripts (self-updating), the existing `shaharia-lab/homebrew-tap`
(`Formula/jev.rb` plus pinned `jev@<version>.rb`, written via the scoped GitHub App token), and
crates.io. Package-manager installs never self-update.
