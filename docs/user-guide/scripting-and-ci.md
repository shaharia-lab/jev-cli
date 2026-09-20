# Scripting and CI

`jev` is built to be called by other programs. Two rules make that predictable:

1. **stdout is data. stderr is everything else**: progress, notices, warnings, logs and errors.
2. **Exit codes are a stable contract.** They change only in a release that says so in the
   [changelog](../../CHANGELOG.md).

The full table, the JSON shapes and the error object are in
[Exit codes and JSON contract](exit-codes.md). This page is the practical side.

## The codes you branch on

| Code | Meaning |
| ---: | --- |
| 0 | Success, and any gate condition holds |
| 2 | Usage or validation error. Nothing was sent, nothing was billed |
| 3 | No API key, or the API refused it |
| 4 | The API rejected the request |
| 5 | Rate limited or overloaded, after retries |
| 6 | Network failure or timeout, after retries |
| 7 | A batch finished, but some rows failed |
| 10 | Evaluated, and the gate condition is false. Never an error |
| 11 | Evaluated, and the answer is inside the abstain band |

Exit 10 is the one to understand. It means the call worked and the answer is no. The answer is
printed as usual and stderr stays empty, so an `if` around the command reads exactly like the
question. Every other non-zero code is a failure: a script must never treat a missing key (3) or
an outage (6) as a "no".

## Gating flags

| Flag | Commands | Exit 10 when |
| --- | --- | --- |
| `--fail-under P` | `jev noul`, `jev score` | The answer is below `P` |
| `--fail-over P` | `jev noul`, `jev score` | The answer is above `P` |
| `--expect OPTION` | `jev choice` | That option did not win. Repeat it to accept several |
| `--min-confidence C` | `jev choice`, `jev score` | The confidence is below `C` |
| `--assert CONDITION` | `jev eval` | The condition is false. Repeat it and all must hold |
| `--abstain-band LO,HI` | `jev noul` | Exit **11**: the probability is inside the band |

A condition that could never be decided, such as an unknown question id or an option that does
not exist, is exit 2 and is caught before anything is sent, so a broken gate costs nothing.

## Three outcomes from one call

```bash
jev noul "Is this ticket about billing?" --state-file ticket.txt --fail-under 0.7
case $? in
  0)  echo "billing" ;;
  10) echo "not billing" ;;
  *)  echo "jev failed" >&2; exit 1 ;;
esac
```

With `set -e`, capture the status so exit 10 does not end the script:

```bash
status=0
jev noul "Is this message spam?" --state-file message.txt \
  --fail-under 0.6 --abstain-band 0.4,0.6 --field noul > probability.txt || status=$?
case "$status" in
  0)  echo "spam" ;;
  10) echo "not spam" ;;
  11) echo "unclear, send it to a person" ;;
  *)  echo "jev failed with exit $status" >&2; exit "$status" ;;
esac
```

## Getting one value out

`--field` prints one raw value and nothing else, which is usually what a script wants:

```bash
probability=$(jev noul "Is this spam?" --state-file message.txt --field noul)
team=$(jev eval -f triage.yaml --state-file ticket.txt --field answers.department.choice)
```

For the whole document, ask for JSON explicitly rather than relying on the pipe:

```bash
jev eval -f triage.yaml --state-file ticket.txt -o json | jq '.answers.is_urgent.noul'
```

`-o` takes `table`, `json`, `yaml` or `jsonl`, and `JEV_OUTPUT` sets it for a whole session.
`--quiet` silences the notices on stderr.

## Gating on one answer of many

```bash
jev eval -f triage.yaml --state-file ticket.txt --assert 'is_urgent >= 0.7'
jev eval -f triage.yaml --state-file ticket.txt --assert 'department == billing' --assert 'department.confidence >= 0.8'
```

## In CI

```yaml
- name: Check the commit message
  env:
    TYPESAFE_API_KEY: ${{ secrets.TYPESAFE_API_KEY }}
  run: |
    git log -1 --pretty=%B > message.txt
    jev noul "Does this commit describe a user-facing change?" \
      --state-file message.txt --model jev-1.13.0 --fail-under 0.7 --quiet || exit_code=$?
    if [ "${exit_code:-0}" = "10" ]; then echo "no changelog entry needed"; fi
```

Four things to get right in a CI job:

- **Put the key in a secret** and export it as `TYPESAFE_API_KEY`. `jev` never accepts a key as a
  flag value, so it cannot leak into a build log or a process list.
- **Pin a versioned model** such as `jev-1.13.0`. An alias moves without notice, and only a
  versioned id has a known price.
- **Let `CI=true` do its work.** Most CI systems set it, and `jev` then never prompts and never
  updates itself in the background. `--no-input` and `JEV_NO_INPUT` do the same anywhere else.
- **Do not wrap `jev` in a retry loop.** It already retries rate limits, timeouts and server
  errors. Exits 2, 3 and 4 will fail the same way every time.

## Errors, in one JSON object

When output is machine-readable, a failure prints exactly one JSON object on stderr and nothing
on stdout:

```json
{
  "error": {
    "code": "no_api_key",
    "exit_code": 3,
    "message": "no API key is configured for profile `default`",
    "hint": "create a key at https://console.typesafe.ai/keys, then either `export TYPESAFE_API_KEY=...` or store it with `jev auth login`",
    "retryable": false
  }
}
```

Branch on `code`, show `message`, and follow `hint` before retrying anything. `jev schema error`
prints its JSON Schema.

## Discovering the contract from a script

```bash
jev spec -o json | jq -r '.commands[] | select(.implemented) | .path'
jev spec -o json | jq '.exit_codes'
jev schema output -o json > output.schema.json
```

`jev spec` prints every command, flag, default, exit code and example, so tooling can build
command lines without scraping help text. The same content, written for a person, is the
[command reference](../commands.md).
