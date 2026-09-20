# jev documentation

`jev` is an unofficial, open-source command-line tool for TypeSafe AI's **Jev** model. Jev never
writes text. You give it some content and typed questions, and it answers with calibrated
probabilities, so your script, your CI job or your AI agent can act on the answer.

New here? Start with the [quick start](user-guide/quick-start.md). It takes about a minute and
costs a fraction of a cent.

> **Unofficial project.** `jev` is community-built and is not affiliated with, endorsed by or
> sponsored by TypeSafe AI. "TypeSafe" and "Jev" belong to their owner.

## User guide

Read these to use `jev`.

| Page | What it covers |
| --- | --- |
| [Installation](user-guide/installation.md) | Install script, Homebrew, cargo, verifying a download, completions and man pages |
| [Quick start](user-guide/quick-start.md) | Your API key, a first question, a first script, a first request file |
| [The three question types](user-guide/question-types.md) | When to ask a noul, a choice or a score, and how to read each answer |
| [Writing good questions](user-guide/writing-questions.md) | What Jev is good at, what belongs in your code, and the habits that raise accuracy |
| [Scripting and CI](user-guide/scripting-and-ci.md) | Exit codes as branches, gating flags, `--field`, JSON output, GitHub Actions |
| [Exit codes and JSON contract](user-guide/exit-codes.md) | Every exit code, the result envelope, the error object, batch records |
| [Many rows at once](user-guide/batch.md) | `jev batch run` over JSONL and CSV: cost first, concurrency, resume, summaries |
| [Configuration and profiles](user-guide/configuration.md) | Settings and precedence, the config file, profiles, the API key, environment variables |
| [AI agents and MCP](user-guide/mcp.md) | `jev mcp serve`, client setup, spend caps, the agent skill |
| [Keeping jev up to date](user-guide/updates.md) | `jev update`, background updates, pinning a version, rollback |
| [Troubleshooting](user-guide/troubleshooting.md) | The errors people actually hit, and what to do about each |
| [FAQ](user-guide/faq.md) | Cost, accuracy, privacy, Jev versus an LLM, and other common questions |

**[Command reference](commands.md)** lists every command, flag, default, exit code and example.
It is generated from the binary, so it always matches the version you have installed.
`jev <command> --help` prints the same facts for one command, and `jev spec` prints all of it as
JSON for tooling.

## Developer guide

Read these to work on `jev`.

| Page | What it covers |
| --- | --- |
| [Development setup](development/setup.md) | Toolchain, the everyday loop, the quality gates CI runs |
| [Architecture](development/architecture.md) | The two crates, how a command runs, where to add things |
| [Testing](development/testing.md) | What each test file guards, mocks, snapshots, fuzzing, live tests |
| [Releasing](development/release.md) | How a release is cut, signed, verified and published |
| [How the API actually behaves](development/api-behaviour.md) | Where the server differs from its documentation, limits, pricing, size estimation |
| [Threat model](development/threat-model.md) | What is protected, which inputs are untrusted, where each protection lives |

[CONTRIBUTING.md](../CONTRIBUTING.md) is the contribution workflow, and
[CLAUDE.md](../CLAUDE.md) (to which `AGENTS.md` is a symlink) is the full architecture guide that
contributors and AI agents both work from. Report security problems privately as described in
[SECURITY.md](../SECURITY.md).
