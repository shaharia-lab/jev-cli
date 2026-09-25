# AI agents and MCP

`jev` was built for AI agents as much as for people. An agent can use it in two ways: as a
command it runs in a shell, or as an [MCP](https://modelcontextprotocol.io) server its client
connects to.

| Use | When |
| --- | --- |
| The CLI | Anything a person will keep or rerun: scripts, CI steps, Makefiles, request files under version control, bulk work |
| The MCP server | A judgment the agent needs during a conversation: structured arguments, no shell quoting, no files |

## Why an agent can drive `jev` without being told how

- `jev spec` prints every command, flag, default, exit code and example as one JSON document.
- `jev <command> --help` says what the command is for, *when to use it rather than its siblings*,
  its inputs, its output contract, its exit codes and two to four copy-pasteable examples.
- `jev schema request|questions|batch-record|output|error` prints JSON Schemas for everything
  `jev` reads and writes.
- `jev validate` checks a request file offline, for free, and reports every problem at once with
  a rule id and a suggested fix.

An agent that reads only the help text can call the command correctly. That is a rule of the
project, and a test enforces it.

## Running the MCP server

```bash
jev mcp serve
```

It speaks JSON-RPC over stdio: one message per line on stdout, logs and notices on stderr only.
Give it a key with `jev auth login` or `TYPESAFE_API_KEY` in the environment the client starts it
from, so the key never appears in a client's configuration file.

### Claude Code

```bash
claude mcp add jev -- jev mcp serve
```

### Claude Desktop, Cursor, and a project's `.mcp.json`

```json
{
  "mcpServers": {
    "jev": { "command": "jev", "args": ["mcp", "serve"] }
  }
}
```

Claude Desktop reads `claude_desktop_config.json`; Cursor reads `~/.cursor/mcp.json`, or
`.cursor/mcp.json` inside a project.

### VS Code (`.vscode/mcp.json`)

```json
{
  "servers": {
    "jev": { "type": "stdio", "command": "jev", "args": ["mcp", "serve"] }
  }
}
```

If the client cannot find `jev`, use the full path that `command -v jev` prints.

## The tools

| Tool | What it does |
| --- | --- |
| `evaluate` | One state, many questions, one call |
| `noul` | One yes/no question |
| `choice` | One question with options |
| `score` | One question with a rubric |
| `validate` | Check a request offline. Free, and works without a key |
| `list_models` | The models the API offers |
| `batch_run` | Run a question set over a file. Offered only with `--allow-dir` |

Every result is the same JSON envelope that `jev eval -o json` prints, plus the session's call
count and estimated spend. A failure comes back as a tool error carrying the same JSON error
object the CLI prints on stderr, so an agent can read `code` and `hint` and fix its own mistake.
Every request is validated before anything is sent.

## Capping what an agent can spend

```json
{
  "mcpServers": {
    "jev": {
      "command": "jev",
      "args": ["mcp", "serve", "--model", "jev-1.13.0", "--max-cost-usd-per-call", "0.001"]
    }
  }
}
```

`--max-cost-usd-per-call` refuses a call whose estimated cost is higher. A capped server needs a
pinned model, because an alias has no published price. Add `--profile <name>` to use a profile's
key and settings.

## Giving the agent files

`batch_run` is offered only when the server is started with one or more `--allow-dir`:

```bash
jev mcp serve --model jev-1.13.0 --allow-dir /path/to/data \
  --max-batch-rows 1000 --max-batch-cost-usd 0.05
```

Inside those directories and nowhere else. Every path is resolved before use, symbolic links and
`..` included, the output file must not exist yet, and a run over `--max-batch-rows` or
`--max-batch-cost-usd` is refused before anything is sent. Without `--allow-dir`, the server
touches no file at all.

The input is a JSONL or CSV file, or a JSON array (`input_format: "json"`, at most 50 MB), with
the same format detection as `jev batch run --input`.
`merge: true` adds each input row to its record as `row`, as `jev batch run --merge` does.

## The agent skill

[`skills/jev-cli`](../../skills/jev-cli/SKILL.md) teaches an agent the workflow: discover
commands with `jev spec`, write request files from `jev schema`, check them with `jev validate`,
run them with `-o json`, branch on exit codes, gate scripts on an answer, label records in bulk,
and choose between the CLI and the MCP server. It also covers writing questions Jev answers well.

**Claude Code plugin**

```bash
claude plugin marketplace add shaharia-lab/jev-cli
claude plugin install jev@jev-cli
```

**Any other agent**, through [skills.sh](https://skills.sh). It installs into the current project;
add `-g` for every project:

```bash
npx skills add shaharia-lab/jev-cli --skill jev-cli
```

The skill needs `jev` on the `PATH` and a key in `TYPESAFE_API_KEY` or from `jev auth login`.

## What the server will not do

- It never prints the API key, in any result, log or error.
- It writes nothing to stdout except protocol messages, so a stray log line cannot corrupt the
  session.
- It touches no file unless you passed `--allow-dir`.
- It contacts no host other than the configured TypeSafe base URL.

The reasoning is in the [threat model](../development/threat-model.md).
