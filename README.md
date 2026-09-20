# jev

An unofficial command-line tool for [TypeSafe AI](https://typesafe.ai)'s **Jev** model, written in
Rust and built for people, shell scripts and AI agents alike.

> [!IMPORTANT]
> **Unofficial project.** `jev` is community-built. It is not affiliated with, endorsed by, or
> sponsored by TypeSafe AI. "TypeSafe" and "Jev" belong to their owner.

> [!WARNING]
> **Before 1.0.** Every command works and the whole surface is tested, but flags, JSON output and
> exit codes may still change in a minor release; [CHANGELOG.md](CHANGELOG.md) calls it out when
> they do. Pin a version if you need the contract to hold.

## What it does

Jev never generates text. You send it some content (the *state*) and typed questions, and it
returns calibrated probabilities over answers you defined. That makes it a semantic `if`
statement, which fits the shell well:

```bash
# Exit status 0 when the answer is yes with probability >= 0.7, 10 when it is not
if git log -1 --pretty=%B | jev noul "Does this commit describe a user-facing change?" --fail-under 0.7; then
  echo "changelog entry needed"
fi
```

What `jev` adds around the API:

- `jev eval`, `jev noul`, `jev choice` and `jev score`, with answers mapped to [exit
  codes](docs/exit-codes.md) a script can branch on
- offline validation before anything is sent or billed, and `--dry-run` cost estimates
- `jev batch run` over JSONL and CSV with bounded concurrency, back-off when rate limited and
  `--resume` after a crash or Ctrl-C
- readable output on a terminal, JSON when piped, and one JSON error object on stderr
- for AI agents: intent-oriented help, a machine-readable command spec, JSON Schemas and an MCP
  server mode
- the API key from `TYPESAFE_API_KEY` or a private credentials file, no telemetry, signed and
  verified self-updates

**Documentation:** [command reference](docs/commands.md) · [exit codes and JSON
contract](docs/exit-codes.md) · [configuration, profiles and the API
key](docs/configuration.md) · [security policy](SECURITY.md)

## Install

Linux and macOS, into `~/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.sh | sh
```

Windows (PowerShell), into `%LOCALAPPDATA%\Programs\jev`:

```powershell
irm https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.ps1 | iex
```

The scripts install the latest stable release. To install a particular one from the
[releases page](https://github.com/shaharia-lab/jev-cli/releases), including a pre-release:
`... | sh -s -- --version 0.1.0`, or in PowerShell set `$env:JEV_INSTALL_VERSION = '0.1.0'`
before running the script. Other options: `--install-dir <dir>`, `--no-modify-path` (the scripts
otherwise add the directory to `PATH` in your shell profile, or your user `PATH` on Windows) and
`--require-signature`; `sh install.sh --help` explains them.

They never use `sudo` or ask for elevation. Before installing anything they check the archive's
SHA-256 against the release's `SHA256SUMS` and, when [minisign](https://jedisct1.github.io/minisign/)
is installed, the signatures of both against the release keys, and they say which checks ran.
A script install updates itself with `jev update`.

Homebrew, which also installs the man pages and shell completions:

```bash
brew install shaharia-lab/tap/jev
```

With Rust's tooling, from [crates.io](https://crates.io/crates/jev-cli) (stable releases only):
`cargo binstall jev-cli` downloads the release archive for your platform and checks its minisign
signature, and `cargo install jev-cli --locked` builds it from source, on any platform Rust
supports. Either way cargo owns the install: `jev update` names the command that updates it
instead of replacing the binary. The library behind `jev` is published as
[`jev-client`](https://crates.io/crates/jev-client).

To check a download yourself — every asset comes with a SHA-256 in `SHA256SUMS` and a minisign
signature — follow [Verifying a release](SECURITY.md#verifying-a-release), which lists the public
keys and the exact commands.

## Quick start

Takes about a minute, and costs a fraction of a cent.

**1. Give `jev` a key.** Create one in the [TypeSafe console](https://console.typesafe.ai/keys),
then either export it, which is what CI jobs and agents should do:

```bash
export TYPESAFE_API_KEY=...
```

or store it once in a private file (mode `0600`), so you do not have to:

```bash
jev auth login
jev auth status
```

**2. Ask a yes/no question.**

```bash
jev noul "Is this message angry?" --state "You charged me twice. Fix it now."
```

```text
answer  noul  yes  0.91  ██████████████████░░
model jev-1.13.0 (requested jev-latest) · 42 input tokens · est. cost $0.000002 · 1 ms · request req_demo
```

Piped or redirected, the same command prints JSON instead, and `--field noul` prints `0.91` and
nothing else.

**3. Use the answer in a script.** A gating flag turns the answer into an exit code: 0 when the
condition holds, 10 when it does not, and something else only when the call itself failed.

```bash
if jev noul "Is this message angry?" --state-file message.txt --fail-under 0.7 --quiet; then
  echo "escalate"
fi
```

**4. Ask several questions about one state**, in one call, from a request file. Write
`triage.yaml`:

```yaml
model: jev-latest
questions:
  is_urgent:
    type: noul
    instructions: Does this ticket convey urgency?
    criteria:
      "true": Explicitly time-sensitive
      "false": No urgency expressed
  department:
    type: choice
    instructions: Which team should handle this?
    criteria:
      billing: Payments, invoicing, refunds
      technical: Bugs, outages, integrations
      other: ~
  frustration:
    type: score
    instructions: How frustrated is the customer?
    criteria:
      - Calm
      - Frustrated
      - Very angry
```

Check it without spending anything, then run it:

```bash
jev validate -f triage.yaml --state-file ticket.txt
jev eval -f triage.yaml --state-file ticket.txt
jev eval -f triage.yaml --state-file ticket.txt --field answers.department.choice
```

`jev schema questions -o json` prints the JSON Schema of such a file, so an editor can check it
as you type, and `jev eval --dry-run` shows the exact request body and what it should cost.

## The three question types

| Type | Command | Answer | Ask it when |
| --- | --- | --- | --- |
| `noul` | `jev noul` | The probability of yes, 0 to 1 | One property holds or it does not. Several nouls can all be high, so ask one per label. |
| `choice` | `jev choice` | One of your options, with confidence and per-option probabilities | Exactly one of up to 255 options must win. |
| `score` | `jev score` | A position on your rubric, with confidence | A described scale of 2 to 10 levels: how much, not which one. |

```bash
jev choice "Which team should handle this?" --state-file ticket.txt \
  --option billing="Payments, invoicing, refunds" --option technical="Bugs and outages" --option other

jev score "How frustrated is the customer?" --state-file ticket.txt \
  --level Calm --level Frustrated --level "Very angry"
```

Jev reads literally: it cannot count, do arithmetic or compare dates, so keep those in code. A
probability near 0.5 means the model cannot tell, not "medium" — `--abstain-band 0.4,0.6` turns
that into exit 11 so a person can look.

## In scripts and CI

stdout is data; stderr is progress, notices, warnings and errors. Exit codes are a stable
contract — the full table, the gating flags and the JSON shapes are in
[docs/exit-codes.md](docs/exit-codes.md).

| Code | Meaning |
| ---: | --- |
| 0 | Success, and any gate condition holds |
| 2 | Usage or validation error; nothing was sent |
| 3 | No API key, or the API refused it |
| 4 | The API rejected the request |
| 5 | Rate limited or overloaded, after retries |
| 6 | Network failure, timeout or server error, after retries |
| 7 | A batch finished, but some rows failed |
| 10 | Evaluated, and the gate condition is false — never an error |
| 11 | Evaluated, and the answer is inside the abstain band |

```bash
# One call, three outcomes
jev noul "Is this ticket about billing?" --state-file ticket.txt --fail-under 0.7
case $? in
  0)  echo "billing" ;;
  10) echo "not billing" ;;
  *)  echo "jev failed" >&2; exit 1 ;;
esac

# Gate on one answer of a multi-question request
jev eval -f triage.yaml --state-file ticket.txt --assert 'is_urgent >= 0.7'
```

In CI, set `TYPESAFE_API_KEY` as a secret and pin a versioned model such as `--model jev-1.13.0`:
an alias moves without notice, and only a versioned id has a known price. `jev` never prompts
when stdin is not a terminal, when `--no-input` or `JEV_NO_INPUT` is set, or when `CI=true`.

## Many rows at once

`jev batch run` applies one question set to every row of a JSONL or CSV file, a few at a time,
and writes one JSON record per row:

```bash
# What it would cost, before sending anything
jev batch run -f triage.yaml --input tickets.jsonl --state-field text --id-field id --dry-run

jev batch run -f triage.yaml --input tickets.jsonl --state-field text --id-field id \
  --out results.jsonl --concurrency 4 --summary-json summary.json

# Continue after a crash or Ctrl-C: rows already answered are not sent again
jev batch run -f triage.yaml --input tickets.jsonl --state-field text --id-field id \
  --out results.jsonl --resume
```

Every row is validated before the first request. A failed row becomes an `error` record and does
not stop the run (`--fail-fast` and `--max-errors` change that), and a run with any failed row
exits 7. Rate limiting pauses every worker for the server's `retry-after` and then spaces
requests out again. The summary goes to stderr, and to `--summary-json`.

## Configuration, profiles and the API key

A value comes from a flag, else an environment variable, else the selected profile, else the
built-in default. `jev config list` shows every setting and where its value came from;
[docs/configuration.md](docs/configuration.md) has the full list, the file format and the
platform paths.

```bash
jev config set model jev-1.13.0        # a default for this profile
jev config get model                   # the value in effect, and where it comes from
jev config path                        # where the configuration file is

jev profile create ci --model jev-1.13.0 --output json
jev auth login --profile ci            # a key of its own
jev noul "Is this spam?" --state "$MESSAGE" --profile ci
```

The key lives in `TYPESAFE_API_KEY`, which always wins, or in the `credentials` file that
`jev auth login` writes (mode `0600`, in a `0700` directory). There is no OS keychain and no
browser flow, the key is never accepted as a flag value, and only its last four characters are
ever shown.

## Use it from an AI agent (MCP)

`jev mcp serve` runs `jev` as a [Model Context Protocol](https://modelcontextprotocol.io) server
over stdio. It offers six tools: `evaluate` (a state and many questions in one call), `noul`,
`choice` and `score` (one question each), `validate` (offline, free, needs no key) and
`list_models`. Each result is the same JSON envelope `jev eval -o json` prints, plus `session`,
the calls made and the estimated spend so far; each failure is a tool error carrying the JSON error
object `jev` prints on stderr. Every request is validated before anything is sent.

Give it a key once with `jev auth login` (or set `TYPESAFE_API_KEY` in the environment the client
starts it from), so that the key never has to appear in a client's configuration. If the client
cannot find `jev` on its `PATH`, use the full path that `command -v jev` prints.

**Claude Code**

```bash
claude mcp add jev -- jev mcp serve
```

**Claude Desktop** (`claude_desktop_config.json`), **Cursor** (`~/.cursor/mcp.json`, or
`.cursor/mcp.json` in a project) and a project's **`.mcp.json`**:

```json
{
  "mcpServers": {
    "jev": { "command": "jev", "args": ["mcp", "serve"] }
  }
}
```

**VS Code** (`.vscode/mcp.json`):

```json
{
  "servers": {
    "jev": { "type": "stdio", "command": "jev", "args": ["mcp", "serve"] }
  }
}
```

To cap what an agent can spend, pin a versioned model (an alias such as `jev-latest` has no known
price, so a capped server refuses it) and set a per-call limit:
`"args": ["mcp", "serve", "--model", "jev-1.13.0", "--max-cost-usd-per-call", "0.001"]`. Add
`"--profile", "<name>"` to use a profile's key and settings. Logs go to stderr; stdout carries only
protocol messages.

A seventh tool, `batch_run`, runs `jev batch run` over a JSONL or CSV file and writes the records
to a new file. It is offered only when the server is started with one or more `--allow-dir <dir>`,
and it reads and writes nothing outside them: every path is resolved, `..` and symbolic links
included, before it is used. Cap it with `--max-batch-rows` and `--max-batch-cost-usd`; a run over
either is refused before anything is sent. It reports progress to clients that ask for it and
returns the run's summary:
`"args": ["mcp", "serve", "--model", "jev-1.13.0", "--allow-dir", "/path/to/data", "--max-batch-rows", "1000", "--max-batch-cost-usd", "0.05"]`.

## Agent skill

[`skills/jev-cli`](skills/jev-cli/SKILL.md) is an [agent skill](https://agentskills.io) that
teaches an AI agent to use `jev` from a shell: discover commands with `jev spec`, write request
files from `jev schema`, check them with `jev validate`, run them with `-o json`, branch on exit
codes, gate scripts on an answer, label records in bulk with `jev batch run`, and choose between
the CLI and the MCP server. It also covers how to write questions Jev answers well. For deeper
question design, use TypeSafe's own skill, [typesafe-ai/skills](https://github.com/typesafe-ai/skills).

**Claude Code plugin**

```bash
claude plugin marketplace add shaharia-lab/jev-cli
claude plugin install jev@jev-cli
```

**Other agents**, through [skills.sh](https://skills.sh) (project-local by default; add `-g` for
every project):

```bash
npx skills add shaharia-lab/jev-cli --skill jev-cli
```

The skill needs `jev` on the `PATH` and an API key from `TYPESAFE_API_KEY` or `jev auth login`.

## JSON Schemas

`jev schema request|questions|batch-record|output|error` prints a JSON Schema (draft 2020-12)
generated from the types `jev` itself reads and writes: a complete request, a question set (a
request file whose state comes from elsewhere), one line of `jev batch run` output, the result of an
evaluation, and the error object printed on stderr. The same
files are in [`schemas/`](schemas). To have an editor check request files, point the YAML or JSON
language server at one, for example with a first line in a YAML file:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/schemas/questions.schema.json
```

## Shell completions and man pages

Installed with Homebrew (`brew install shaharia-lab/tap/jev`), the man pages and the Bash, Zsh
and Fish completions are set up for you. Otherwise, `jev completion <shell>` prints a completion
script for the commands and flags of the `jev` you run. Enable it once per shell, and run it again
after upgrading:

| Shell | Enable it |
| --- | --- |
| Bash | add `source <(jev completion bash)` to `~/.bashrc` |
| Zsh | add `source <(jev completion zsh)` to `~/.zshrc`, after `compinit` |
| Fish | `jev completion fish > ~/.config/fish/completions/jev.fish` |
| PowerShell | add `jev completion powershell \| Out-String \| Invoke-Expression` to `$PROFILE` |

Man pages (`man jev`, `man jev-noul`, ...) are generated from the same definitions as `--help`.
From a checkout, `make dist-assets` writes them and the four completion scripts to
`target/dist-assets/`; `MANPATH="$PWD/target/dist-assets/man:$MANPATH" man jev` reads them.

## Updates

`jev update` moves to the latest release now; `jev update --check` only asks (exit 20 when a newer
one exists), and `jev update --rollback` undoes the last update. Every download is checked against
signing keys compiled into `jev` before anything is replaced.

A `jev` installed with the install script also updates itself. At most once a day, after a command
has finished, a detached process looks for a newer release on GitHub and stages it; the next command
swaps it in and says so in one line on stderr. A command's output and exit code never change, and it
never waits for the check. (On Windows the check starts only from a command run at a terminal,
because a process started there would otherwise keep a caller's pipe open.) `jev version` shows whether this happens and, if not, why. It is off
for Homebrew and cargo installs (update those with `brew upgrade` or `cargo install`), when
`CI=true`, when `jev` cannot write to its own directory, and when you turn it off:

```bash
jev config set update.auto false            # for good
export JEV_AUTO_UPDATE=false                 # for one environment
jev config set update.pin_version 0.3.1      # stay on one version
jev config set update.channel prerelease     # follow pre-releases too
```

## Security and privacy

- **No telemetry, ever.** `jev` contacts exactly two hosts: the configured TypeSafe base URL
  (`https://api.typesafe.ai` by default) when you run a command that evaluates something, and
  GitHub Releases of this repository when it checks for an update. Nothing else, and nothing in
  the background beyond that update check, which you can turn off.
- **Your data stays yours.** The `state` is sent to the API and nowhere else. `jev` never writes
  request or response bodies to a log unless you pass `--debug-bodies`, and never stores them.
- **The API key** is held in memory in a type that wipes itself, is redacted in every log and
  error, is never accepted as a flag value, and never appears in `--dry-run` output or in an MCP
  result. Only its last four characters are ever shown.
- **`https://` only**, except loopback: a plain `http://` base URL needs `--insecure-allow-http`.
  TLS uses `rustls` with the platform's certificate store, certificate verification cannot be
  disabled, and redirects are never followed.
- **Updates are verified**: a minisign signature and a SHA-256 checksum, both against keys
  compiled into the binary, before anything is replaced; `jev` never downgrades on its own.
- **The MCP server touches no file** unless you start it with `--allow-dir`, and then only inside
  those directories.

[`SECURITY.md`](SECURITY.md) has the full model and how to report a vulnerability privately.
Please do not open a public issue for one.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) to build and test, and [`CLAUDE.md`](CLAUDE.md) for the
project guide. Report security problems privately as described in [`SECURITY.md`](SECURITY.md).

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without
any additional terms or conditions.
