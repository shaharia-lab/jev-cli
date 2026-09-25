# Many rows at once

`jev batch run` asks the same questions about every row of a JSONL or CSV file, or of a JSON
array, and writes one JSON record per row. It is the right tool for labelling tickets, messages, reviews, documents or
any export you have lying around.

## The short version

```bash
# 1. Check the question set. Free, no key needed
jev validate -f triage.yaml --strict

# 2. See what the run would cost, before sending anything
jev batch run -f triage.yaml --input tickets.jsonl --state-field body --model jev-1.13.0 --dry-run

# 3. Try ten rows and read them
jev batch run -f triage.yaml --input tickets.jsonl --state-field body --model jev-1.13.0 \
  --id-field ticket_id --limit 10 --out sample.jsonl

# 4. Run the file
jev batch run -f triage.yaml --input tickets.jsonl --state-field body --model jev-1.13.0 \
  --id-field ticket_id --out results.jsonl --summary-json summary.json
```

Always do steps 1 and 2. A dry run needs a pinned model, because an alias has no published
price.

## Choosing what to send

| Flag | What each row sends as the state |
| --- | --- |
| nothing | The whole row |
| `--state-field body` | One field of the row |
| `--state-fields subject,body` | An object of only those fields |

`--id-field ticket_id` keys the records by a field of your own, which is what makes the output
joinable back to your data. Without it, records are keyed by line number (for a JSON array, the
element's 1-based index). Ids must be unique,
and a repeated one is caught before anything is sent.

CSV files need a header row; `--input-format` is only needed when the extension does not say
what the file is. Rows can also be piped in as JSONL with `--input -`.

A file holding one JSON array, such as an API dump, is read with `--input-format json`: each
element is a row, exactly as one JSONL line is. A `.json` file whose content is one array is read
that way without the flag; any other `.json` file is still read as JSONL. An array is read whole,
so it must be a file of at most 50 MB and cannot come from stdin. For a bigger array, or one
nested inside an object, convert it to JSONL first, which streams with no limit:

```bash
jev batch run -f triage.yaml --input items.json --state-fields author,text --id-field key
jq -c '.items[]' dump.json > items.jsonl
```

## Everything is checked first

For a file, `jev batch run` validates the question set and reads every row before it sends
anything: a malformed row, a missing field or a repeated id is exit 2, and nothing was billed.
Piped rows can only be read once, so they are checked as they arrive and sending stops at the
first bad one.

## The output

One JSON record per row, written whole with its newline, in the order rows finish (`--ordered`
for input order):

```json
{"id":"t-1001","status":"ok","model":"jev-1.13.0","answers":{"is_urgent":{"type":"noul","noul":0.94}},"usage":{"input_tokens":459,"output_tokens":73},"cost_usd":0.0000193,"request_id":"req_01hq","latency_ms":412}
{"id":"t-1002","status":"error","error":{"code":"api_rejected","exit_code":4,"message":"...","retryable":false}}
```

A row that fails becomes an `error` record and does not stop the run, so one bad row cannot cost
you the whole file. The run exits 7 when any row failed. To find them:

```bash
jq -r 'select(.status == "error") | .id' results.jsonl
```

`jev schema batch-record` prints the schema of a record. The end-of-run summary goes to stderr,
and to `--summary-json`: rows, failures, tokens, estimated cost, wall time, retries and why the
run stopped.

## Resuming

After exit 7 (some rows failed) or 130 (you pressed Ctrl-C), fix the cause and run the same
command again with `--resume`:

```bash
jev batch run -f triage.yaml --input tickets.jsonl --state-field body --model jev-1.13.0 \
  --id-field ticket_id --out results.jsonl --resume
```

There is no state file to manage. The output *is* the state: rows already recorded `ok` are not
sent again, rows recorded `error` are retried, and a half-written line left by a crash is
removed before anything is appended.

## Speed, cost and rate limits

- `--concurrency` defaults to 4. Above roughly 8, a shared key gets rate limited, which makes
  the run slower rather than faster.
- When the server does rate limit, every worker pauses for the time it asks for and then spaces
  requests out again. You do not need a retry loop of your own.
- `--fail-fast` stops at the first failure, `--max-errors N` after N of them. Requests already in
  flight still finish and are recorded.
- Memory does not grow with the file. Rows are streamed, and only their ids are remembered.
- Ctrl-C stops sending, gives requests in flight up to ten seconds to be recorded, and exits 130.
  A second Ctrl-C exits at once.

## A worked example

`tickets.jsonl`, one ticket per line:

```json
{"ticket_id":"t-1001","subject":"Charged twice","body":"You charged me twice this month. Fix it now."}
{"ticket_id":"t-1002","subject":"How do I export?","body":"Where can I download my invoices?"}
```

Run the triage question set from the [quick start](quick-start.md) over it, then turn the
records into a CSV of the routing decision:

```bash
jev batch run -f triage.yaml --input tickets.jsonl --state-fields subject,body \
  --id-field ticket_id --model jev-1.13.0 --out results.jsonl

jq -r 'select(.status == "ok") | [.id, .answers.department.choice, .answers.is_urgent.noul] | @csv' results.jsonl
```

The full flag list, including `--skip-size-check` and `--warn-unpinned`, is in the
[command reference](../commands.md#jev-batch-run).
