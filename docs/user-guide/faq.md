# FAQ

## What is Jev, in one sentence?

A model that never writes text: you give it some content and typed questions, and it returns
calibrated probabilities over answers you defined.

## How is that different from asking an LLM?

An LLM writes a sentence you then have to parse, and it can decide to phrase things differently
tomorrow. Jev returns a number in a shape you specified, so your code can branch on it. It is
also much faster and much cheaper, because it generates nothing.

It is not a replacement for an LLM. It is the piece you reach for when the job is a decision:
classify, route, detect, rate, verify, gate.

## What does it cost?

Only input tokens are billed. At the time of writing that is $0.042 per million input tokens for
`jev-1.13.0`, so a short ticket costs a small fraction of a cent. `jev` always labels a cost as
an estimate, and `--dry-run` prices a run before you send it.

## Is this the official TypeSafe CLI?

No. `jev` is an independent, open-source project and is not affiliated with, endorsed by or
sponsored by TypeSafe AI. "TypeSafe" and "Jev" belong to their owner. You need a TypeSafe API
key to use it.

## Does it send my data anywhere else?

No. `jev` contacts exactly two hosts: the TypeSafe base URL you configured, when you run a
command that evaluates something, and GitHub Releases of this repository, when it checks for an
update, which you can turn off. There is no telemetry of any kind. Your state goes to the API and
nowhere else, and is never written to a log unless you pass `--debug-bodies`.

## Where is my API key kept?

In `TYPESAFE_API_KEY`, or in a `credentials` file with mode `0600` in a `0700` directory. There
is no OS keychain, no OAuth and no browser flow, by design. The key is never accepted as a flag
value, is held in memory in a type that wipes itself, and only its last four characters are ever
shown. See [Configuration and profiles](configuration.md#the-api-key).

## Are the answers deterministic?

Not bit for bit. Compare against thresholds, never for equality. Pin a versioned model such as
`jev-1.13.0` for anything repeatable: the alias `jev-latest` moves to new versions without
notice.

## What does a probability of 0.5 mean?

That the model cannot tell, not that the answer is "medium". Use `--abstain-band 0.4,0.6` to exit
11 for those cases and send them to a person.

## What is `confidence`, then?

How peaked the distribution is, not how likely the answer is to be right. Two genuinely
acceptable options give a low confidence, and that is fine.

## Can I use it without a key?

Yes, for everything offline: `jev validate`, `jev schema`, `jev spec`, `jev completion`,
`jev version` and any command with `--dry-run`. None of them touch the network.

## How do I use it in CI?

Put the key in a secret, export it as `TYPESAFE_API_KEY`, pin a versioned model, and branch on
the exit code. `CI=true` already stops `jev` prompting and updating itself. See
[Scripting and CI](scripting-and-ci.md).

## How many rows can `jev batch run` handle?

As many as you have. Rows are streamed, so memory does not grow with the file, and only ids are
kept. Concurrency defaults to 4 and shared keys get rate limited above roughly 8. A crash or a
Ctrl-C is not a lost run: `--resume` picks up from the output file.

## Can an AI agent use it?

That is half the point. Every command's help says when to use it, `jev spec` prints the whole
contract as JSON, `jev schema` prints JSON Schemas, and `jev mcp serve` exposes the same
functionality as MCP tools. See [AI agents and MCP](mcp.md).

## Which platforms are supported?

Linux (x86-64, arm64), macOS (Intel and Apple silicon) and Windows (x86-64, arm64). One binary,
no runtime to install.

## Is it stable? Can flags change?

Flags, JSON shapes and exit codes are versioned like an API and follow SemVer. Before 1.0 a
breaking change can land in a minor release, and the [changelog](../../CHANGELOG.md) always says
so. Pin a version if you need the contract to hold exactly.

## How do I turn off the background update check?

`jev config set update.auto false`, or `JEV_AUTO_UPDATE=false` for one environment. It is
already off for Homebrew and cargo installs and when `CI=true`. See
[Keeping jev up to date](updates.md).

## Something is broken. Where do I report it?

Issues and feature requests: <https://github.com/shaharia-lab/jev-cli/issues>. Security problems
go privately through [SECURITY.md](../../SECURITY.md), never a public issue. Start with
[Troubleshooting](troubleshooting.md), which covers most of what goes wrong.
