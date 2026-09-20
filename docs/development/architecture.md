# Architecture

A map of the codebase: the two crates, what happens when a command runs, and where to put a new
thing. [CLAUDE.md](../../CLAUDE.md) is the long form, kept next to the code and binding on
contributors and AI agents alike. This page is the orientation.

## Two crates, one direction

```text
jev-cli  ──depends on──>  jev-client
```

**`crates/jev-client`** is a standalone library: typed requests and answers (`serde` plus
`schemars`), offline validation, the HTTP transport (`reqwest` with `rustls`), SDK-parity
retries, typed errors and pricing. It knows nothing about `clap`, terminals or configuration
files, and CI enforces that boundary. It is published on crates.io and usable on its own.

**`crates/jev-cli`** is the `jev` binary: the `clap` command tree, one module per command group
under `commands/`, configuration and profiles, credential storage, output renderers, the batch
engine, the MCP server and the updater.

Both crates set `#![forbid(unsafe_code)]`.

## What happens when a command runs

1. **Parse.** `cli.rs` holds the whole command tree. A command carries only its one-line purpose
   and its arguments; the rest of its help lives in `help/docs.rs`, which `help::apply` attaches
   to the tree, so `--help` and `jev spec` cannot disagree.
2. **Resolve settings.** `config::Settings::resolve` applies one precedence rule everywhere:
   flag, then environment variable, then profile, then default. A command reads
   `context.settings()`; it never reads a flag or an environment variable itself.
3. **Prepare.** `evaluate::prepare` resolves the model, fills in the state, validates offline and
   builds the typed request. Nothing is sent before validation passes, and `--dry-run` stops
   here.
4. **Send.** `evaluate::send` makes one call through a `Transport` and wraps the answer in the
   result envelope. Every path that evaluates anything, including the shortcuts, the batch engine
   and the MCP server, goes through these same two steps, so they cannot drift apart.
5. **Render.** A command never prints. It returns a value implementing `output::Render`, and
   `Output::emit` decides the format: text on a terminal, JSON when piped, or whatever `-o` and
   `--field` ask for.
6. **Fail.** Every failure is a `CliError`. `error.rs` holds the only mapping from a library error
   to an exit code, and `lib.rs` prints it once: text for a person, one JSON object for a program.

## The pieces worth knowing

| Area | Where | Note |
| --- | --- | --- |
| Validation | `jev-client`, `validate/` | Works on the raw document, so every problem is reported at once with a rule id and a JSON Pointer |
| Transport | `jev-client`, `http.rs` | The only code that reaches the API. rustls, no redirects, key scrubbed from server text |
| Settings | `jev-cli`, `config/` | One resolver, one writer (`ConfigStore::update`), comments preserved |
| Credentials | `jev-cli`, `credentials.rs` | Environment variable or one `0600` file. No keychain, by decision |
| Output | `jev-cli`, `output/` | Four formats, `--field`, and control characters escaped before anything is printed |
| Gates | `jev-cli`, `gate.rs` | Exit 10 means exactly one thing: evaluated, condition false |
| Batch | `jev-cli`, `batch/` | Knows nothing of `clap`, so the MCP server drives it too. Streams rows, memory is O(concurrency) |
| MCP | `jev-cli`, `commands/mcp/` | Hand-rolled JSON-RPC over stdio; the loop is the only writer to stdout |
| Schemas | `jev-cli`, `schemas.rs` | Generated from the types that read and write the data. Never write a schema by hand |
| Updater | `jev-cli`, `update/` | Verifies signatures and checksum before replacing anything, and never downgrades on its own |

## Rules that shape the code

- **Humans and AI agents are equal users.** Every command's help states its purpose, when to use
  it rather than its siblings, its inputs, its output contract, its exit codes and two to four
  examples. A lint test fails when one is missing.
- **stdout is data, stderr is everything else.** Nothing else may write to stdout.
- **Never prompt** when stdin is not a terminal, when `--no-input` or `JEV_NO_INPUT` is set, or
  when `CI=true`. `Interaction` is the only code that decides this.
- **Never read stdin** unless it is the only possible source. A caller that leaves stdin open
  would otherwise hang, and agents often do.
- **Validate before spending.** The server does not enforce its own limits reliably, so
  client-side validation is a correctness feature, not a convenience.
- **Exit codes, JSON shapes and flags are versioned API.** Changing one is a breaking change.
- **The key never appears anywhere.** Not in arguments, logs, errors, dry-run output or MCP
  results. `tests/secret_leak.rs` is the net under every per-feature check.
- **Nothing `jev` did not write reaches a terminal unescaped.** `output::printable` turns control
  characters into escapes, including in the `tracing` diagnostics behind `-v`.

## Adding something

| You are adding | Do this |
| --- | --- |
| A command | Arguments in `cli.rs`, a module under `commands/`, routing in `commands::run`, a `Doc` in `help/docs.rs`, a scenario in `tests/secret_leak.rs`, then `make reference` |
| A flag that is also a setting | A `Key` in `config/` (name, parser, default, flag, environment variable) and a row in the precedence table test |
| A validation rule | A variant of `Rule`, an entry in `Rule::ALL`, and a case in `tests/validation.rs`; a test fails if a rule has no case |
| An MCP tool | A constructor in `commands/mcp/tools.rs` and a line in `tools::all()`; a tool not listed there cannot be called |
| A parser of outside input | A fuzz entry point in that crate's `src/fuzz.rs` and a target under `fuzz/` |

After any change to a command, a flag or its help, run `make reference`, and
`JEV_UPDATE_SNAPSHOTS=1 cargo test -p jev-cli --all-features --test help` to refresh the help
snapshots. Review both diffs.

## Further reading

- [How the API actually behaves](api-behaviour.md): the server quirks several rules exist for.
- [Threat model](threat-model.md): what is protected and where.
- [Testing](testing.md): what each test file guards.
- [CLAUDE.md](../../CLAUDE.md): the full guide, including the reasoning behind each rule above.
