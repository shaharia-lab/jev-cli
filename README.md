# jev

An unofficial command-line tool for [TypeSafe AI](https://typesafe.ai)'s **Jev** model, written in
Rust and built for people, shell scripts and AI agents alike.

> [!IMPORTANT]
> **Unofficial project.** `jev` is community-built. It is not affiliated with, endorsed by, or
> sponsored by TypeSafe AI. "TypeSafe" and "Jev" belong to their owner.

> [!WARNING]
> **Early development.** There is no release yet and the commands below are not implemented.
> Progress is tracked in the [v1 epic](https://github.com/shaharia-lab/jev-cli/issues/2).

## What it will do

Jev never generates text. You send it some content (the *state*) and typed questions, and it
returns calibrated probabilities over answers you defined. That makes it a semantic `if`
statement, which fits the shell well:

```bash
# Exit status 0 when the answer is yes with probability >= 0.7, 10 when it is not
if git log -1 --pretty=%B | jev noul "Does this commit describe a user-facing change?" --fail-under 0.7; then
  echo "changelog entry needed"
fi
```

Planned for the first version:

- `jev eval`, `jev noul`, `jev choice`, `jev score` with answers mapped to exit codes
- Offline validation before anything is sent or billed
- `jev batch run` over JSONL and CSV with bounded concurrency, pool-wide back-off when rate
  limited, `--dry-run` cost estimates, and `--resume` after a crash or Ctrl-C
- Human output on a terminal, JSON when piped, and stable exit codes
- For AI agents: intent-oriented help, a machine-readable command spec, JSON Schemas, and an MCP
  server mode
- API key from `TYPESAFE_API_KEY` or a private credentials file, no telemetry, signed and verified self-updates

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

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) to build and test, and [`CLAUDE.md`](CLAUDE.md) for the
project guide. Report security problems privately as described in [`SECURITY.md`](SECURITY.md).

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without
any additional terms or conditions.
