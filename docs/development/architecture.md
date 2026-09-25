# Architecture

The detailed map of the codebase. [CLAUDE.md](../../CLAUDE.md) is the constitution and the short
version; this page is where the reasoning lives. Read the section for the area you are about to
change before you change it.

## Two crates, one direction

```text
jev-cli  ──depends on──>  jev-client
```

**`crates/jev-client`** is a standalone library: typed requests and answers (`serde` plus
`schemars`), offline validation, the HTTP transport (`reqwest` with `rustls`), SDK-parity
retries, typed errors and pricing. It knows nothing about `clap`, terminals or configuration
files, and CI enforces that boundary (`scripts/ci/check-client-deps.sh`). It is published on
crates.io and usable on its own.

**`crates/jev-cli`** is the `jev` binary: the `clap` command tree, one module per command group
under `commands/`, configuration and profiles, credential storage, output renderers, the batch
engine, the MCP server and the updater.

Both crates set `#![forbid(unsafe_code)]`. Commands depend on traits (`Transport`,
`CredentialStore`, `Clock`, `UpdateSource`) so they can be tested with fakes. The library uses
`thiserror`; the binary has exactly one error to exit-code mapping.

## What happens when a command runs

1. **Parse.** `cli.rs` holds the whole command tree. A command carries only its one-line purpose
   and its arguments; the rest of its help is a `Doc` in `help/docs.rs`, which `help::apply`
   attaches to the tree, so `--help` and `jev spec` cannot disagree.
2. **Resolve settings.** `config::Settings::resolve` applies one precedence rule everywhere:
   flag, then environment variable, then profile, then default. A command reads
   `context.settings()?`; it never reads a flag or an environment variable for a setting itself.
3. **Prepare.** `evaluate::prepare` resolves the model (flag, request file, environment,
   profile, default), fills in the state, validates offline and builds the typed request.
4. **Send.** `evaluate::send` makes one call through a `Transport` and wraps the answer in the
   `ResultEnvelope`.
5. **Render.** A command never prints. It returns a value implementing `output::Render`
   (`Serialize` for the machine formats, plus a hand-written human form) and `Output::emit` does
   the rest: `--field`, the four formats, text on a terminal versus JSON on a pipe.
6. **Fail.** Every failure is a `CliError`. `CliError::from(jev_client::Error)` in `error.rs` is
   the only mapping to exit codes, and `lib.rs` prints it once: text for a person, one JSON
   object for a program.

The shortcut commands, the batch engine and the MCP server all build a `Document` and go through
`prepare` and `send`, so they cannot drift apart. Nothing is sent before validation passes, and
`--dry-run` never touches the network and never needs a key.

The key comes from a `CredentialStore`. The transport is built in `client.rs`, which is where the
HTTPS rule is enforced: settings only check that a base URL is well formed, because whether plain
`http://` is acceptable depends on `--insecure-allow-http`.

**Never read stdin unless it is the only possible source.** A caller that leaves stdin open would
make `jev` hang, and agents often do. A request file that carries its own `state` therefore never
reads stdin; `--state-file -` is the explicit way to ask for it.

## The typed model (`jev-client`)

Three rules hold the types together:

- **Requests are sent as written.** Key order, explicit `null` and unknown fields are all
  preserved.
- **Responses parse tolerantly.** Unknown fields are ignored, and an unknown or reshaped answer
  becomes `Answer::Unknown` carrying its raw JSON.
- **Every type derives `JsonSchema`,** and the request schemas are deliberately stricter than the
  parser: they reject unknown fields outright.

A type whose doc comment contains rustdoc links or examples sets
`#[schemars(description = "...")]`, because agents read those descriptions, and a test enforces
it. Public structs are `#[non_exhaustive]` with constructors, so adding a field is not a breaking
change.

## Offline validation

`jev_client::validate` checks the raw `Document`, not a typed `Request`. That is deliberate: it
lets every problem be reported at once with a question id, a JSON Pointer path, a stable rule id
and a fix, and it lets duplicate keys be reported, which any ordinary parse would hide.

Adding a rule means adding it to `Rule`, to `Rule::ALL`, and to `rule_cases()` in
`tests/validation.rs`. A test fails if any rule has no case.

The size estimate is per character class, not a flat ratio, and
`crates/jev-client/tests/fixtures/token-calibration.json` holds it to real token counts. Re-probe
and update that file rather than loosening the bound.

## The transport

`HttpTransport` is the only code that touches the network for the API. TLS is `rustls` with the
`ring` provider and the platform certificate verifier, built explicitly: no process-global
provider, no `aws-lc`, no OpenSSL. Redirects are never followed. Server-supplied error text is
scrubbed of the key and never quotes a body of unknown shape, because a 422 echoes the `state`.

Tests run against `wiremock` with a fake `Clock`, so nothing sleeps.
`crates/jev-client/tests/http_transport.rs` holds the sentinel-key leak test, which must keep
passing at `TRACE` level.

## Output, and never letting foreign text reach a terminal

A human form never prints text `jev` did not write itself as it stands. `output::printable`
writes control characters out as escapes, so a server's message, a model card, a question id
echoed from a request file or a batch row cannot drive the terminal. `Ui`'s styling methods and
`Cell` do this for whatever they are given, which covers most values; anything interpolated into
a human string without them applies `printable` itself, and the escaped text is what `Cell`
measures, so the columns still line up.

Machine formats escape control characters themselves and must stay byte-faithful, and so must the
single raw values that `--field`, `jev config get` and `jev config path` print for a script.

The `tracing` diagnostics behind `-v` and `-vv` follow the same rule through a `FormatFields` in
`logging.rs` that runs every recorded value through `printable`, so a new `tracing::debug!` in
either crate is safe without doing anything. `crates/jev-cli/tests/escape.rs` sends a sentinel
escape sequence to each of these surfaces.

`Interaction` is the only code that decides whether a prompt is allowed.

## Help, and why it is a feature

In `cli.rs` a command has only its one-line purpose (a one-paragraph doc comment) and its
arguments. Everything else is its `Doc`: when to use it rather than its siblings, input, output,
exit codes, and two to four examples, one of them machine-readable.

The lint test in `help/lint.rs` fails, naming what is missing, when a command with real arguments
has no complete doc, when help text is written in `cli.rs` instead, or when an example does not
parse against the real tree or would wrap. A `Pending` placeholder is exempt until it gets
arguments.

Every `--help` and `jev spec` is a snapshot in `crates/jev-cli/tests/snapshots/`. After an
intended change:

```bash
JEV_UPDATE_SNAPSHOTS=1 cargo test -p jev-cli --all-features --test help
```

## Gates and exit codes

`gate.rs` turns an answer into an exit code. **Exit 10 means exactly one thing: evaluated
successfully, condition false.** It is returned as `Ok(Exit::GateFalse)`, never as an error: the
answer is printed normally and stderr stays empty. Anything that goes wrong, including an answer
the gate cannot compare, is a `CliError` with its own code.

Conditions are checked against the request before sending (unknown question id, a choice compared
with `>=`, an option that does not exist), so a gate that could never be decided costs nothing.

The shortcut commands flatten their one answer to the top level, which is what makes
`--field noul`, `--field choice` and `--field score` work. `jev eval` adds `gate` to its envelope
only when `--assert` was used.

## The API key

TypeSafe has one way to authenticate a developer: a bearer API key. `jev` finds it in exactly two
places: `TYPESAFE_API_KEY`, which always wins, or the `credentials` file that `jev auth login`
writes once (mode `0600` from its first byte, in a `0700` directory, refused with the exact
`chmod` when others can read it). That is all of `credentials.rs`.

**There is no OS keychain, by the owner's decision: do not add one back.** Do not add OAuth, SSO
or any browser flow either. Only the last four characters of a key are ever shown
(`fingerprint`), and a parse error of the credentials file is never shown, because the message
would quote a line that holds a key.

## Settings and the configuration file

One resolver, `config::Settings::resolve`: **flag > environment > profile > default**, each value
carrying its `Source`. Adding a setting means adding a `Key` (name, parser, default, flag,
environment variable) and a row in the precedence table test.

A key with a `table()` (`update.auto`, `update.channel`, `update.pin_version`) is stored once in
that top-level table for every profile, and its source is `config` rather than `profile`.

The configuration is loaded lazily as a `Result`, so a broken `config.toml` stops only the
commands that need it: `jev version` and `jev config path` must keep working.
`ConfigStore::update` is the only way to write: lock, re-read, change, temp file, rename. Edits go
through `toml_edit` and must preserve a person's comments, and an existing file is never given
keys nobody asked for.

Warnings are `Notice`s on stderr: text for a person, one JSON line for a program, nothing under
`--quiet`. Read the environment through `Env`, not `std::env`, so it can be tested.

## The batch engine

`batch/` knows nothing of `clap`, so the MCP server can drive it too.

- Rows are read lazily on a thread of their own and handed to the pool through a channel with one
  slot per worker, so memory is O(concurrency), never O(rows). Only ids are remembered, as
  16-byte fingerprints.
- The exception is a JSON array (`RowFormat::Json`): it is one value, so it is parsed whole and
  then handed out element by element. `MAX_JSON_INPUT_BYTES` (50 MB) bounds that memory, and the
  error points at JSONL, which streams. Detecting a `.json` array parses it without keeping it
  (`IgnoredAny`), so a file of JSON lines that start with `[` stays JSONL.
- A file is read twice: once to check every row (mapping, repeated ids) before anything is sent,
  then to send. Piped rows can be read only once, so they are checked as they arrive.
- Records are written to stdout as the run goes, one per row, flushed at once, in completion
  order. `--ordered` holds records back behind a slower earlier row, within a bounded window.
- A row that fails is a record, not an error. The run exits 7 when any did.
- The engine takes no signals and prints nothing. It is given an `interrupt` future (the
  command's is `interrupt::first_of_two`: SIGINT or SIGTERM, exit 130, a second signal exits at
  once) and reports `Event`s to an observer that draws the progress.
- `--resume` has no state file. The output is the state (`batch/resume.rs`): ids recorded `ok`
  are skipped, and an incomplete last line left by a crash is truncated away before appending.
- Pool-wide back-off is `jev_client::Throttle`, attached to the one transport every worker
  shares.

## The MCP server

`commands/mcp/` is a hand-rolled JSON-RPC loop over stdio, not an SDK: one message per line, and
that loop is the only writer to stdout, so logs and notices can only reach stderr.

- Tools live in `commands/mcp/tools.rs`. Adding one means a constructor and a line in
  `tools::all()`; a tool that is not listed there cannot be called.
- Every evaluating tool goes through `Session::evaluate`: `prepare`, the
  `--max-cost-usd-per-call` check, then `send`.
- Input schemas are generated from the `jev-client` types with every definition inlined.
- A tool that fails returns a tool error (`isError`) carrying `CliError::to_json()`. Only an
  unknown method or tool is a JSON-RPC error.
- The key and settings are read once at start-up, and a server without a key still starts, so
  that `validate` works.
- `batch_run` (`commands/mcp/batch_run.rs`) is listed only with `--allow-dir`, and opens files
  only through `roots::Roots`: every path is canonicalised and must land inside a root, an output
  is created with `create_new` in a canonicalised parent, and after opening the path is resolved
  again, so a directory swapped for a symlink is caught. It prices every row as `--dry-run` does
  and refuses a run over `--max-batch-rows` or `--max-batch-cost-usd` before creating the output.
- A tool reports progress through the `Reporter` it is handed, the one other writer to stdout,
  and only when the call carried a `progressToken`.

## JSON Schemas

Schemas come from one place, `schemas.rs`, generated from the types that read and write the data
(`Request`, `ResultEnvelope`, `batch::Record`, `ErrorDocument`). `jev schema` prints them, and the
MCP tool input schemas are built from the same generator. **Never write a schema by hand.**

`schemas/` at the repository root holds the published copies and doubles as their snapshot: after
an intended change run `make schemas` and review the diff, because a test fails when they are
stale. Schema-only constraints (limits, `required` on fields serialized as `null`) go on the type
with `#[schemars(extend(...))]`; `#[schemars(required)]` on an `Option` drops `null` from the
type.

## The updater

`update/` only ever reads GitHub Releases of this repository, through `UpdateSource`
(`GitHubReleases`, or a fake in tests).

- `stage` downloads into memory with size caps and verifies the minisign signatures of
  `SHA256SUMS` and of the archive against the two keys in `crates/jev-cli/keys/` (`include_str!`,
  either may sign), **and** that each trusted comment is exactly `file:<name>\tversion:<version>`,
  and then the archive's SHA-256. Only then is the binary written to `.jev-update/` beside the
  executable.
- `Installation::apply` keeps the old binary as `.jev-update/previous`, renames the new one in (on
  Windows the running `.exe` is renamed aside first), runs its self-test (`jev version -o json`)
  and puts the old one back if that fails.
- Automatic paths never downgrade (`is_upgrade`, `MINIMUM_VERSION`); only `--version` may.
- The install method comes from the path (`Cellar` is Homebrew, `<root>/bin` with `.crates.toml`
  is cargo) or from the install script's receipt, `.jev-update/receipt.json`. A managed install is
  told its package manager's command instead of being replaced.

Automatic updates (`update/auto.rs`, hooks in `commands/update.rs`) run *around* a command in
`lib.rs`, never inside it, and never around `jev update` or `jev version` (the self-test).
`before` swaps in a staged upgrade and prints one `Notice::info` line. `after` claims the day's
check in `auto-update.json` in the configuration directory (a non-blocking lock, so concurrent
runs start one check) and spawns `jev update` with its hidden --background flag, detached with
null stdio, without waiting for it.

The guards, in the order `jev version` reports them (`update::guard`): package manager,
unreadable configuration, `JEV_AUTO_UPDATE` or `update.auto`, `update.pin_version`, `CI=true`, no
install receipt, directory not writable. Only an install with the receipt updates itself, so
development builds and tests never do unless a test creates one. Failures are recorded in the
state file and logged at `-v`, never shown otherwise.

Tests serve signed fake releases from `wiremock` through the test hooks `JEV_TEST_UPDATE_URL`,
`JEV_TEST_UPDATE_KEY` and `JEV_TEST_VERSION`, which release builds do not contain.

## Generated artefacts

Completion scripts (`jev completion`, via `clap_complete`) and man pages (`man.rs`, via
`clap_mangen`) are rendered from `cli::command()` with help attached, so they follow the tree with
no extra work when a command is added. `make dist-assets` (`examples/dist-assets.rs`) writes both
for release archives and packages; it runs on the build host, so it serves cross-compiled targets
too.

`docs/commands.md` is generated the same way: `reference.rs` renders the Markdown from the
`CommandTree` behind `jev spec`, `make reference` writes the file, and a test fails when the
committed copy is stale. It leaves the version out on purpose, so a release cannot make it stale.

## The agent skill

`skills/jev-cli/SKILL.md` teaches *users'* agents, and installs with `npx skills add` or as a
Claude Code plugin (`.claude-plugin/marketplace.json`, whose plugin is that one directory). It
teaches a workflow and defers the contract to `jev spec`, `--help` and `jev schema`.

`crates/jev-cli/tests/skill.rs` fails when it names a command or flag that `jev spec` lacks or
marks pending, when its exit-code table or MCP tool list differs from the binary's, or when its
request file does not validate. Renaming any of these means editing the skill in the same pull
request. Its question-writing advice is our own words: never paste TypeSafe's documentation into
it.

## Adding something

| You are adding | Do this |
| --- | --- |
| A command | Arguments in `cli.rs`, a module under `commands/`, routing in `commands::run`, a `Doc` in `help/docs.rs`, a scenario in `tests/secret_leak.rs`, then `make reference` |
| Behaviour for a pending command | Replace its `Pending` arguments in `cli.rs`, then the same steps |
| A flag that is also a setting | A `Key` in `config/` (name, parser, default, flag, environment variable) and a row in the precedence table test |
| A validation rule | A variant of `Rule`, an entry in `Rule::ALL`, and a case in `tests/validation.rs` |
| An MCP tool | A constructor in `commands/mcp/tools.rs` and a line in `tools::all()` |
| A parser of outside input | A fuzz entry point in that crate's `src/fuzz.rs` and a target under `fuzz/` |
| Anything users can see | The matching page in `docs/user-guide/`, and `make reference` if help changed |

## Further reading

- [How the API actually behaves](api-behaviour.md): the server quirks several of these rules
  exist for.
- [Threat model](threat-model.md): what is protected, and where each protection lives.
- [Testing](testing.md): what each test file guards.
- [Releasing](release.md): how a release is built, signed, verified and published.
