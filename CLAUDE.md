# jev-cli — guide for AI agents and contributors

`AGENTS.md` is a symlink to this file. Edit `CLAUDE.md` only.

## What this project is

`jev` is an **unofficial**, open-source Rust CLI for TypeSafe AI's **Jev** model (not affiliated with
TypeSafe AI). Jev is a "System One" model: it never generates text. You send a `state` plus a map of
typed questions (`noul` = yes/no probability, `choice` = one of up to 255 options, `score` = position on
a 2–10 level rubric) and get calibrated probabilities back. The CLI makes that usable from a terminal,
shell scripts and CI (answers become exit codes), bulk jobs, and AI agents (including an MCP server mode).

Status: **early implementation.** `jev-client` has the typed model, offline validation and the HTTP
transport. The `jev` binary has the whole command tree, the output layer, structured errors and the
exit-code contract. `jev eval`, `jev noul`, `jev choice`, `jev score`, `jev validate`, `jev batch run`,
`jev models list`, `jev auth`, `jev config`, `jev profile`, `jev mcp serve`, `jev spec`, `jev schema`, `jev completion`, `jev update` and `jev version` work, and install-script installs update themselves in the background. Work is tracked in epic
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
   an 11-level Score gets only a bare 400), so client-side validation is a correctness feature.
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
reshaped answer becomes `Answer::Unknown` with its raw JSON). Every type derives `JsonSchema`; the
request schemas are deliberately stricter than the parser and reject unknown fields outright. A
type whose doc comment has rustdoc links or examples sets `#[schemars(description = "...")]`, because
agents read those descriptions (a test enforces this). Public structs are `#[non_exhaustive]` with
constructors, so adding a field is not a breaking change.

Offline validation (`jev_client::validate`) checks the raw `Document`, not a typed `Request`, so that
every problem is reported at once with a question id, a JSON Pointer path, a stable rule id and a
fix, and so that duplicate keys (which any ordinary parse hides) can be reported. Adding a rule means
adding it to `Rule`, to `Rule::ALL`, and to `rule_cases()` in `tests/validation.rs`; a test fails if
any rule has no case. The size estimate is per character class, not a flat ratio, and
`tests/fixtures/token-calibration.json` holds it to real token counts: re-probe and update that file
rather than loosening the bound.

`HttpTransport` is the only code that touches the network for the API. TLS is `rustls` with the
`ring` provider and the platform certificate verifier, built explicitly (no process-global provider,
no `aws-lc`, no OpenSSL). Redirects are never followed. Server-supplied error text is scrubbed of the
key and never quotes a body of unknown shape, because a 422 echoes `state`. Tests run against
`wiremock` with a fake `Clock`, so nothing sleeps; `tests/http_transport.rs` holds the sentinel-key
leak test, which must keep passing at `TRACE` level.

In `jev-cli`, a command never prints. It returns a value implementing `output::Render` (`Serialize`
for the machine formats plus a hand-written human form) and `Output::emit` does the rest: `--field`,
the four formats, and text on a terminal versus JSON on a pipe. Every failure is a `CliError`, and
`CliError::from(jev_client::Error)` in `error.rs` is the only mapping to exit codes. Errors are
printed once, in `lib.rs`: text for a person, one JSON object for a program. `Interaction` is the only
code that decides whether a prompt is allowed. To give a pending command its behaviour, replace its
`Pending` arguments in `cli.rs`, add a module under `commands/`, route it in `commands::run`, and
give it a `Doc` in `help/docs.rs`.

Help follows one standard (product rule 1). In `cli.rs` a command has only its one-line purpose (a
one-paragraph doc comment) and its arguments; everything else is its `Doc`: when to use it versus
its siblings, input, output, exit codes, and 2 to 4 examples, one machine-readable. `help::apply`
attaches the docs to the tree that is parsed, so `--help` and `jev spec` cannot disagree. The lint
test in `help/lint.rs` fails, naming what is missing, when a command with real arguments has no
complete doc, when help text is written in `cli.rs` instead, or when an example does not parse
against the real tree or would wrap. A `Pending` placeholder is exempt until it gets arguments. Every
`--help` and `jev spec` are snapshots in `tests/snapshots/`; after an intended change, run
`JEV_UPDATE_SNAPSHOTS=1 cargo test -p jev-cli --all-features --test help` and review the diff.

Every evaluation goes through `evaluate.rs`: `prepare` (resolve the model as flag > request file >
env > profile > default, fill in the state, validate offline, build the typed request) and `send`
(one call through a `Transport`, wrapped in the `ResultEnvelope`). The shortcut commands, batch and
the MCP server must build a `Document` and use the same two steps, so that they cannot drift apart.
Nothing is sent before validation passes, and `--dry-run` never touches the network or needs a key.
The key comes from a `CredentialStore`; the transport is built in `client.rs`, which is also where the
HTTPS rule is enforced (settings only check that a base URL is well-formed, because whether plain
http:// is acceptable depends on `--insecure-allow-http`). **Never read stdin unless it is the only
possible source**: a caller that leaves stdin open would make `jev` hang, and agents often do. A
request file with its own `state` therefore never reads stdin; `--state-file -` is the explicit way.
Integration tests run `jev` against `wiremock` with `TYPESAFE_BASE_URL` pointed at it (loopback
http is allowed), and default to an address nothing listens on so a test can never reach the real API.

`jev mcp serve` (`commands/mcp/`) is a hand-rolled JSON-RPC loop over stdio, not an SDK: one
message per line, and that loop is the only writer to stdout, so logs and notices can only reach
stderr. Tools live in `commands/mcp/tools.rs`; adding one means a constructor and a line in
`tools::all()`, and a tool that is not listed there cannot be called. Every evaluating tool goes
through `Session::evaluate`, which is `prepare`, the `--max-cost-usd-per-call` check, then `send`.
Input schemas are generated from the `jev-client` types with every definition inlined. A tool that
fails returns a tool error (`isError`) carrying `CliError::to_json()`; only an unknown method or
tool is a JSON-RPC error. The key and settings are read once at start-up, and a server without a key
still starts so that `validate` works. `batch_run` (`commands/mcp/batch_run.rs`) is listed only with
`--allow-dir`, and opens files only through `roots::Roots`: every path is canonicalised and must
land inside a root, an output is created with `create_new` in a canonicalised parent, and after
opening, the path is resolved again so a directory swapped for a symlink is caught. It prices every
row as `--dry-run` does and refuses a run over `--max-batch-rows` or `--max-batch-cost-usd` before
creating the output. A tool reports progress through the `Reporter` it is handed, the one other
writer to stdout, and only when the call carried a `progressToken`.

The batch engine (`batch/`) knows nothing of `clap`, so the MCP server can drive it too. Rows are
read lazily on a thread of their own and handed to the pool through a channel with one slot per
worker, so memory is O(concurrency), never O(rows); only ids are remembered, as 16-byte fingerprints.
A file is read twice: once to check every row (mapping, repeated ids) before anything is sent, then
to send. Piped rows can be read only once, so they are checked as they arrive. Like `jev mcp serve`,
`jev batch run` writes to stdout as it goes: one record per row, flushed at once, in completion
order (`--ordered` holds records back behind a slower earlier row, within a bounded window). A row
that fails is a record, not an error; the run exits 7 when any did. The engine takes no signals and
prints nothing: it gets an `interrupt` future (the command's is `interrupt::first_of_two`, SIGINT or
SIGTERM, 130; a second signal exits at once) and reports `Event`s to an observer that draws the
progress. `--resume` has no state file: the output is the state (`batch/resume.rs`), ids recorded
`ok` are skipped, and an incomplete last line from a crash is truncated away before appending.
Pool-wide back-off is `jev_client::Throttle`, attached to the one transport every worker shares.

JSON Schemas come from one place, `schemas.rs`, which generates them from the types that read and
write the data (`Request`, `ResultEnvelope`, `batch::Record`, `ErrorDocument`); `jev schema` prints them and the MCP
tool input schemas are built from the same generator. Never write a schema by hand. `schemas/` at
the repository root holds the published copies and doubles as their snapshot: after an intended
change run `make schemas` and review the diff (a test fails when they are stale). Schema-only
constraints (limits, `required` on fields serialized as `null`) go on the type with
`#[schemars(extend(...))]`; `#[schemars(required)]` on an `Option` drops `null` from the type.

Gates (`gate.rs`) turn an answer into an exit code. **Exit 10 means exactly one thing: evaluated
successfully, condition false.** It is returned as `Ok(Exit::GateFalse)`, never as an error: the
answer is printed normally and stderr stays empty. Anything that goes wrong, including an answer the
gate cannot compare, is a `CliError` with its own code. Conditions are checked against the request
before sending (unknown question id, a choice compared with `>=`, an option that does not exist), so a
gate that could never be decided costs nothing. The shortcuts flatten their one answer to the top
level so `--field noul|choice|score` works; `jev eval` adds `gate` to its envelope only when
`--assert` was used.

TypeSafe has one way to authenticate a developer: a bearer API key. `jev` finds it in exactly two
places: **`TYPESAFE_API_KEY`**, which always wins, or the **`credentials` file** that `jev auth login`
writes once (`0600` from its first byte, in a `0700` directory, refused with the exact `chmod` when
others can read it). That is all of `credentials.rs`. **There is no OS keychain, by the owner's
decision (PRD D9): do not add one back**, and do not add OAuth, SSO or any browser flow. Only the last
four characters of a key are ever shown (`fingerprint`), and a parse error of the credentials file is
never shown, because it would quote a line that holds a key.

Settings come from one resolver, `config::Settings::resolve`: **flag > environment > profile >
default**, each value carrying its `Source`. A command reads `context.settings()?`; it never reads a
flag or an environment variable for a setting itself. Adding a setting means adding a `Key` (name,
parser, default, flag, env var) and a row in the precedence table test. A key with a `table()`
(`update.auto`, `update.channel`, `update.pin_version`) is stored once in that top-level table for
every profile, and its source is `config` rather than `profile`. The configuration is loaded
lazily as a `Result`, so a broken `config.toml` stops only the commands that need it (`jev version` and
`jev config path` must keep working). `ConfigStore::update` is the only way to write: lock, re-read,
change, temp file, rename. Edits go through `toml_edit` and must preserve a person's comments; an
existing file is never given keys nobody asked for. Warnings are `Notice`s on stderr: text for a
person, one JSON line for a program, nothing under `--quiet`. Read the environment through `Env`, not
`std::env`, so it can be tested.

Completion scripts (`jev completion`, via `clap_complete`) and man pages (`man.rs`, via
`clap_mangen`) are rendered from `cli::command()`, help attached, so they follow the tree with no
extra work when a command is added. `make dist-assets` (`examples/dist-assets.rs`) writes both for
release archives and packages; it runs on the build host, so it serves cross-compiled targets too.

Test hooks live behind the `internal-test-hooks` cargo feature (`jev debug render|error|prompt|panic`).
CI turns it on with `--all-features`; a second, default-features test run proves that release builds
do not contain them. Tests must isolate the environment (`CI`, `NO_COLOR`, `TERM`, `LANG`, `JEV_*`,
`TYPESAFE_*`): CI sets `CI=true`, and a developer's shell has a UTF-8 locale. What a person sees is
tested on a real pseudo-terminal in `tests/terminal.rs` (Unix only).

Every parser of outside input is fuzzed (`fuzz/`, cargo-fuzz on nightly, a workspace of its own).
Entry points live in each crate's `src/fuzz.rs` behind `#[cfg(any(test, fuzzing))]`, so they reach
private code and the crates' tests keep them compiling; a new parser gets an entry point and a
target there. A crash is fixed with a regression test next to the code. The performance budgets
(NFR-PERF-1..3) are `#[ignore]`d benchmarks in `tests/performance.rs` and `tests/batch.rs`, run on a
release build by CI's `performance` job (`make bench`); their regression guards are set from CI's
Linux runner, so raise one only after finding out why it tripped.

The agent skill for *users'* agents is `skills/jev-cli/SKILL.md`, installable with `npx skills add`
and as a Claude Code plugin (`.claude-plugin/marketplace.json`, whose plugin is that one directory).
It teaches a workflow and defers the contract to `jev spec`, `--help` and `jev schema`.
`tests/skill.rs` fails when it names a command or flag that `jev spec` lacks or marks pending, when
its exit-code table or MCP tool list differs from the binary's, or when its request file does not
validate: renaming any of these means editing the skill in the same PR. Its question-writing
advice is our own words; never paste TypeSafe's documentation into it.

The updater (`update/`) only ever reads GitHub Releases of this repository, through `UpdateSource`
(`GitHubReleases`, or a fake in tests). `stage` downloads into memory with size caps and verifies
the minisign signatures of `SHA256SUMS` and the archive against the two keys in `keys/`
(`include_str!`, either may sign) **and** that each trusted comment is exactly
`file:<name>\tversion:<version>`, then the archive's SHA-256; only then is the binary written to
`.jev-update/` beside the executable. `Installation::apply` keeps the old binary as
`.jev-update/previous`, renames the new one in (on Windows the running `.exe` is renamed aside
first), runs its self-test (`jev version -o json`) and puts the old one back if it fails. Automatic
paths never downgrade (`is_upgrade`, `MINIMUM_VERSION`); only `--version` may. The install method
comes from the path (`Cellar` is Homebrew, `<root>/bin` with `.crates.toml` is cargo) or the install
script's receipt, `.jev-update/receipt.json`; managed installs get their package manager's command.
Tests serve signed fake releases from `wiremock` through the test hooks `JEV_TEST_UPDATE_URL`,
`JEV_TEST_UPDATE_KEY` and `JEV_TEST_VERSION`, which release builds do not contain.

Automatic updates (`update/auto.rs`, hooks in `commands/update.rs`) run *around* a command in
`lib.rs`, never inside it, and never around `jev update` or `jev version` (the self-test).
`before` swaps in a staged upgrade and prints one `Notice::info` line; `after` claims the day's check
in `auto-update.json` in the config dir (a non-blocking lock, so concurrent runs start one check) and
spawns `jev update --background` (hidden flag) detached with null stdio, not waited for. Guards, in
the order `jev version` reports them (`update::guard`): package manager, unreadable config,
`JEV_AUTO_UPDATE`/`update.auto`, `update.pin_version`, `CI=true`, no install receipt, directory not
writable. Only an install with the receipt updates itself, so dev builds and tests never do unless a
test creates one. Failures are recorded in the state file and logged at `-v`, never shown otherwise.

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
  snapshot tests on stdout, stderr **and** exit code. Live API tests are opt-in only: the smoke suite
  `crates/jev-cli/tests/live.rs` is `#[ignore]`d, runs nightly in `.github/workflows/live-smoke.yml`
  (which files one tracking issue on failure), and asserts the server quirks above so drift is noticed.
- Isolate all test state with `JEV_CONFIG_DIR`; never touch the real user config or credentials.
- Match the surrounding code's naming, idiom and comment density. Document public items in `jev-client`.

## Distribution

GitHub Releases + install scripts (self-updating), the existing `shaharia-lab/homebrew-tap`
(`Formula/jev.rb` plus pinned `jev@<version>.rb`, written via the scoped GitHub App token), and
crates.io. Package-manager installs never self-update.

The install scripts, `install.sh` (POSIX sh) and `install.ps1` (Windows PowerShell 5.1 and 7), live
at the repository root and are served from `main`. They embed the release keys, check the checksum
always and the signature when `minisign` is on PATH, and write `<dir>/.jev-update/receipt.json`
(`installer`, `version`, `target`), which is how `jev update` recognises a self-managed install.
Their release constants (URLs, keys, verifier) sit in one block, one assignment per line:
`tests/install_script.rs` replaces only that block to run them against a local server, and checks
that the shipped values are GitHub over HTTPS and the committed keys. `install.ps1` must stay ASCII.

`release-please.yml` tags and drafts a release as the `jev-release-bot` App (a tag or pull request
made with `GITHUB_TOKEN` triggers no workflow), and the tag starts `release.yml`. It is staged: plan → build
(six native runners) → package (+ CycloneDX SBOM) → publish (sign, draft, `release` environment)
→ attest (build provenance) → verify → finalize → `stable`. Any other ref is a dry run. The
archive layout, the full asset list and the signatures are checked by
`scripts/release/verify-assets.sh`; change it with the matrix. crates.io and the tap hang off
`stable`, which pre-releases skip.

The formulas come from `scripts/release/homebrew-formula.sh` (checksums from the verified
`SHA256SUMS`) and reach the tap through `homebrew-publish.sh` (contents API, idempotent, never moves
`jev.rb` back to an older release). `homebrew-check` runs on every release run of a stable version, dry runs included (a pre-release skips it):
`brew style`, `brew audit --strict`, and `brew install` + `brew test` from that run's archives. The
formula keeps the binary in the keg (`Cellar/jev/<version>/bin/jev`, beside Homebrew's
`INSTALL_RECEIPT.json`), which is how `jev update` recognises a managed install.

crates.io gets `jev-client`, then `jev-cli`, from `scripts/release/crates-publish.sh` (the `crates`
job: `stable`, `release` environment, `CARGO_REGISTRY_TOKEN`), which skips a version already in the
index because a publish can never be undone. `crates/jev-cli` packages only what builds `jev`
(`include`), and `[package.metadata.binstall]` names the release archives and the primary key
(`self-test.sh` holds it to `keys/release-primary.pub`). CI runs `cargo publish --workspace --dry-run`.
`cargo install` and `cargo binstall` both record the install in `<root>/.crates.toml`, which is how
`jev update` recognises them.

Every release asset and `SHA256SUMS` is signed with **minisign** (PRD Q1) by the one release job
that can reach the signing key (the `release` environment: `v*` tags, owner approval). The public
keys are `crates/jev-cli/keys/release-primary.pub` (signs) and `release-next.pub` (rotation); the
updater must trust both and only those. Each signature's trusted comment is
`file:<name>\tversion:<version>`, and verification checks it. `SECURITY.md` lists the keys, and
`scripts/release/self-test.sh` fails when the two disagree.
