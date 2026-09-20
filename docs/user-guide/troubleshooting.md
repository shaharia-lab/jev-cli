# Troubleshooting

Every failure prints a message and a hint, and returns an exit code you can branch on. This page
covers the ones people actually hit. The full contract is in
[Exit codes and JSON contract](exit-codes.md).

## `jev: command not found`

The binary is not on your `PATH`. The install script puts it in `~/.local/bin` and adds that to
your shell profile, which only affects shells started afterwards:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Add that line to your profile, or rerun the installer without `--no-modify-path`. On Windows,
open a new terminal after installing.

## "no API key is configured" (exit 3)

`jev` looks in exactly two places: `TYPESAFE_API_KEY` in the environment, and the `credentials`
file that `jev auth login` writes. Check which one is in play:

```bash
jev auth status --offline
jev config path
```

If you exported the key in another terminal, it is not in this one. If the key exists but the API
refuses it, `jev auth status` says so: the key may have been revoked, or it may belong to another
environment.

## "the credentials file is readable by others"

`jev` refuses to read a key that other users on the machine can read, and prints the exact
`chmod` to run. This usually happens after copying a configuration directory around.

## It asks for input and I am in a script

It should never do that. `jev` does not prompt when stdin is not a terminal, when `--no-input` or
`JEV_NO_INPUT` is set, or when `CI=true`. If you are seeing a prompt, stdin is a terminal:
add `--no-input` and the error will name the flag or variable to use instead.

## It hangs and nothing happens

`jev` reads stdin only when that is the only possible source of the state. A request file that
carries its own `state`, or a `--state` or `--state-file`, is always enough. If a command is
waiting, give it the state explicitly, and use `--state-file -` when you do mean stdin.

## "usage or validation error" (exit 2)

Nothing was sent and nothing was billed. Run the validator, which reports every problem at once:

```bash
jev validate -f triage.yaml --state-file ticket.txt -o json
```

Each finding names the question, the path in the file, a stable rule id and a fix. Common ones:
a score with fewer than 2 or more than 10 levels, a choice with no escape option, a misspelt
field inside a question (the API accepts those silently and the answers quietly get worse), and
duplicate question ids.

## "the API rejected the request" (exit 4)

The request reached the API and it said no. Read the message, then run
`jev validate --strict`. A 422 usually means a value the server does not accept, such as a
`null` score level. Do not retry the same request: it will fail the same way.

## "rate limited" (exit 5)

`jev` already retried and backed off. On a shared key, concurrency above roughly 8 gets rate
limited, so in a batch run lower `--concurrency` before anything else. Otherwise wait and try
again.

## Network failures and timeouts (exit 6)

Also retried already. If it persists, check the base URL:

```bash
jev config get base_url
```

Only `https://` is accepted, except for loopback addresses. A plain `http://` base URL needs
`--insecure-allow-http`, which exists for local mocks and nothing else. Certificate verification
cannot be turned off.

## "a batch finished, but some rows failed" (exit 7)

Expected, and not a lost run. Find the failures, fix the cause, and resume:

```bash
jq -r 'select(.status == "error") | .id' results.jsonl
jev batch run -f triage.yaml --input tickets.jsonl --state-field body --out results.jsonl --resume
```

## The answer looks wrong

- A probability near 0.5 means the model cannot tell, not "medium". Use `--abstain-band` and send
  those cases to a person.
- Check that the question asks for a judgment rather than for counting, arithmetic or a date
  comparison, which Jev cannot do. See [Writing good questions](writing-questions.md).
- A misspelt field inside a question is accepted by the API without complaint and degrades the
  answer. `jev validate --strict` catches it.
- Answers vary slightly between calls. Never compare them for equality.
- If answers changed overnight, an alias moved. Pin a versioned model with
  `jev config set model jev-1.13.0`.

## The wrong model, or the wrong setting

```bash
jev config list
jev config get model
```

Both show where each value came from: a flag, an environment variable, a profile or the default.
`jev config list` is the quickest answer to "why did that run use the wrong model?".

## `jev update` says it cannot update this install

That is a package-managed install, and the message names the right command
(`brew upgrade jev`, or `cargo install jev-cli --locked`). See
[Keeping jev up to date](updates.md).

## Getting more detail

```bash
jev eval -f triage.yaml --state-file ticket.txt -vv
jev eval -f triage.yaml --state-file ticket.txt --dry-run
```

`-v` and `-vv` log to stderr, and never print the key. `--dry-run` shows the exact request body
without sending it. `--debug-bodies` adds request and response bodies to the log: your state is
often customer data, so use it only when you need it and never in CI.

## Still stuck

Open an issue at <https://github.com/shaharia-lab/jev-cli/issues> with the command you ran, the
error, and the output of `jev version`. The `request_id` in an answer or an error helps a great
deal. Never paste your API key, and remember that a state is usually private data.

Security problems go privately through [SECURITY.md](../../SECURITY.md), never a public issue.
