# Exit codes and JSON contract

For people writing shell scripts, CI jobs and agents around `jev`. Exit codes, the JSON that
`jev` prints on stdout, and the JSON error it prints on stderr are a stable contract: after 1.0
they change only in a major release, before it only in a minor release, and
[`CHANGELOG.md`](../CHANGELOG.md) says so when they do.

The command reference in [`commands.md`](commands.md) lists the codes each command can return.
`jev spec` prints the same contract as JSON, and `jev schema output|error|batch-record` prints a
JSON Schema for each shape below.

## Exit codes

| Code | Name | Meaning | What a script should do |
| ---: | --- | --- | --- |
| 0 | `success` | Success, and any gate condition holds. | Continue. |
| 1 | `internal` | A bug in `jev`, or an answer it cannot understand. | Report it; retrying rarely helps. |
| 2 | `usage` | The command line or the request file is wrong. Nothing was sent, nothing was billed. | Fix the command or the file. |
| 3 | `auth` | No API key, or the API refused it. | Set `TYPESAFE_API_KEY` or run `jev auth login`. |
| 4 | `api_rejected` | The API rejected the request (400, 404, 422). | Fix the request; do not retry unchanged. |
| 5 | `rate_limited` | Rate limited or overloaded, after retries. | Back off and try again later; lower `--concurrency` in a batch. |
| 6 | `network` | Network failure, timeout or server error, after retries. | Retry later. |
| 7 | `batch_partial` | A batch finished, but some rows failed. | Read the records; rerun the failed rows with `--resume`. |
| 10 | `gate_false` | Evaluated successfully, and the gate condition is false. | Take the "no" branch. **Never an error.** |
| 11 | `abstain` | Evaluated successfully, and the answer is inside `--abstain-band`. | Ask a person. |
| 20 | `update_available` | `jev update --check` found a newer release. | Optional: run `jev update`. |
| 130 | `interrupted` | Interrupted by Ctrl-C (SIGINT) or SIGTERM. | Stop. |

Exit 10 is the point of the gating flags: the command worked, and the answer says no. The answer
is printed as usual and stderr stays empty, so an `if` around the command reads as the question
does. Anything that goes wrong, including an answer the gate cannot compare, gets its own code
above.

```bash
# Three outcomes from one call: yes, no, and something went wrong
jev noul "Is this ticket about billing?" --state-file ticket.txt --fail-under 0.7
case $? in
  0)  echo "billing" ;;
  10) echo "not billing" ;;
  *)  echo "jev failed" >&2; exit 1 ;;
esac
```

## Gating flags

| Flag | Commands | Exit 10 when |
| --- | --- | --- |
| `--fail-under P` | `jev noul` | The probability of yes is below `P`. |
| `--fail-over P` | `jev noul` | The probability of yes is above `P`. |
| `--expect OPTION` | `jev choice` | That option did not win; repeat the flag to accept any of several. |
| `--fail-under N` | `jev score` | The score is below `N`. |
| `--fail-over N` | `jev score` | The score is above `N`. |
| `--min-confidence C` | `jev choice`, `jev score` | The answer's confidence is below `C`. |
| `--assert CONDITION` | `jev eval` | The condition over one question's answer is false, e.g. `--assert 'is_urgent >= 0.7'`; repeat it and all must hold. |
| `--abstain-band LO,HI` | `jev noul` | (Exit **11**) The probability is inside the band, where the model cannot tell. |

A condition that could never be decided — an unknown question id, a choice compared with `>=`, an
option that does not exist — is exit 2, checked before anything is sent, so it costs nothing.

## Where the output goes

**stdout is data. stderr is everything else**: progress, notices, warnings, logs and errors. A
command prints readable text when stdout is a terminal and JSON when it is piped or redirected;
`-o table|json|yaml|jsonl` (or `JEV_OUTPUT`) says so explicitly, and `--quiet` silences the
notices on stderr.

`--field PATH` prints one raw value and nothing else, which is what a script usually wants:

```bash
p=$(jev noul "Is this spam?" --state "$MESSAGE" --field noul)
jev eval -f questions.yaml --state-file ticket.txt --field answers.department.choice
```

## The result envelope

`jev eval` prints its answers under `answers`, keyed by the question ids in the request file:

```json
{
  "model": "jev-1.13.0",
  "requested_model": "jev-latest",
  "answers": {
    "is_urgent": { "type": "noul", "noul": 0.94 },
    "department": {
      "type": "choice",
      "choice": "billing",
      "confidence": 0.89,
      "probabilities": { "billing": 0.93, "technical": 0.07, "other": 0.0 }
    }
  },
  "usage": { "input_tokens": 459, "output_tokens": 73 },
  "cost_usd": 0.0000193,
  "request_id": "req_01hq",
  "latency_ms": 412
}
```

`jev noul`, `jev choice` and `jev score` ask one question, so they put its fields at the top level
instead, which is what makes `--field noul` work:

```json
{
  "type": "noul",
  "noul": 0.91,
  "model": "jev-1.13.0",
  "requested_model": "jev-latest",
  "usage": { "input_tokens": 42, "output_tokens": 7 },
  "cost_usd": 0.000002,
  "request_id": "req_01hq",
  "latency_ms": 380
}
```

- `model` is the versioned id that answered (`jev-1.13.0`); `requested_model` is what you asked
  for (`jev-latest` moves without notice, so record both).
- `cost_usd` is always an **estimate**: only input tokens are billed, and an alias has no known
  price, so it can be `null`.
- `request_id` is the API's `x-typesafe-request-id`, worth logging and quoting in a support
  request.
- `gate` appears only when a gating flag was used:

```json
{
  "gate": {
    "passed": false,
    "abstained": false,
    "conditions": [{ "condition": "answer >= 0.95", "passed": false, "actual": 0.91 }]
  }
}
```

Parse tolerantly: fields may be added in a minor release, and an answer type `jev` does not know
is passed through as the API sent it.

## The error object

When a command fails and the output is machine-readable, stderr carries exactly one JSON object.
Nothing else is printed on stderr, and stdout stays empty:

```json
{
  "error": {
    "code": "no_api_key",
    "exit_code": 3,
    "error_type": null,
    "message": "no API key is configured for profile `default`",
    "hint": "create a key at https://console.typesafe.ai/keys, then either `export TYPESAFE_API_KEY=...` or store it with `jev auth login`",
    "request_id": null,
    "http_status": null,
    "retryable": false,
    "details": null
  }
}
```

`code` is a stable name to branch on, `exit_code` is the process exit code, `hint` is the next
action to take, and `details` carries structure such as the findings of a failed validation. The
API key never appears in a message, a hint or a log.

## Batch records

`jev batch run` writes JSON Lines: one record per input row, in completion order (`--ordered` for
input order), each written whole with its newline so the file stays valid JSONL even after a
crash.

```json
{"id":"t-1001","status":"ok","model":"jev-1.13.0","answers":{"is_urgent":{"type":"noul","noul":0.94}},"usage":{"input_tokens":459,"output_tokens":73},"cost_usd":0.0000193,"request_id":"req_01hq","latency_ms":412}
{"id":"t-1002","status":"error","error":{"code":"api_rejected","exit_code":4,"message":"...","retryable":false}}
```

The run's summary goes to stderr as one `{"summary": ...}` line (and to `--summary-json`); a run
with any failed row exits 7.

## Validating your own tooling

The shapes above are published as JSON Schemas, generated from the types `jev` reads and writes:

```bash
jev schema output -o json > output.schema.json
jev schema error -o json > error.schema.json
jev schema batch-record -o json > batch-record.schema.json
```

The same files are committed in [`schemas/`](../schemas), and `jev spec` prints every command,
flag, default and exit code as JSON for tooling that builds command lines.
