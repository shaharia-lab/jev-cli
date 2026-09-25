<!-- Generated from the `jev` command tree by `make reference`. Do not edit by hand. -->

# jev command reference

Ask TypeSafe AI's Jev model typed questions from a terminal, a script or an AI agent (unofficial)

jev sends a `state` (text or structured data) and typed questions to TypeSafe AI's Jev model and returns calibrated probabilities, never generated text.

Three question types: `noul` (yes/no, the probability of yes), `choice` (one of up to 255 options) and `score` (a position on a rubric of 2 to 10 levels).

stdout carries data only: readable text on a terminal, JSON when piped. Everything else goes to stderr. Exit codes are a stable contract (listed under Exit codes).

Agents and tooling: `jev spec` prints every command, flag, exit code and example as JSON, and `jev <command> --help` says when to use each command.

This is an unofficial community tool, not affiliated with TypeSafe AI.

This page is generated from the command tree `jev` parses with, so it says what `jev --help` and `jev spec` say. `jev spec` prints the same content as JSON, for tooling.

## Examples

```bash
# Ask a yes/no question about a file; exit 10 when the answer is probably no
jev noul "Is the customer asking for a refund?" --state-file ticket.txt --fail-under 0.7

# Every command, flag and exit code as JSON, for agents and tooling
jev spec | jq -r '.commands[].path'
```

## Exit codes

The whole contract. Each command lists the codes it can return; exit 1 is left out below, because any command can return it.

| Code | Name | Meaning |
| ---: | --- | --- |
| 0 | `success` | success, and any gate condition holds |
| 1 | `internal` | internal error: a bug in jev, please report it |
| 2 | `usage` | usage or validation error; nothing was sent |
| 3 | `auth` | no API key, or the API refused it |
| 4 | `api_rejected` | the API rejected the request |
| 5 | `rate_limited` | rate limited or overloaded, after retries |
| 6 | `network` | network failure, timeout or server error, after retries |
| 7 | `batch_partial` | a batch finished, but some rows failed |
| 10 | `gate_false` | evaluated, and the gate condition is false (never an error) |
| 11 | `abstain` | evaluated, and the answer is inside the abstain band |
| 20 | `update_available` | a newer version is available |
| 130 | `interrupted` | interrupted |

## Global flags

Accepted by every command, before or after its name.

_Global options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--profile <NAME>` | string |  | `JEV_PROFILE` |  | Named profile to use (its API key, base URL and defaults) |
| `-o, --output <FORMAT>` | `table`, `json`, `yaml`, `jsonl` |  | `JEV_OUTPUT` | `output` | Output format |
| `--field <PATH>` | string |  |  |  | Print one raw value from the output, e.g. `answers.is_urgent.noul` |
| `--model <MODEL>` | string | `jev-latest` | `TYPESAFE_DEFAULT_MODEL` | `model` | Model alias or versioned id, e.g. `jev-latest` or `jev-1.13.0` |
| `--base-url <URL>` | string | `https://api.typesafe.ai` | `TYPESAFE_BASE_URL` | `base_url` | API root |
| `--timeout <DURATION>` | duration | `30s` |  | `timeout` | Time allowed per attempt, e.g. `30`, `30s`, `500ms` or `2m` |
| `--max-retries <N>` | integer | `2` |  | `max_retries` | Retries after the first attempt; 0 disables retrying |
| `--no-input` | flag | `false` | `JEV_NO_INPUT` |  | Never ask for input; fail with an error naming the flag to use instead |
| `--no-color` | flag | `false` |  |  | Disable colour (also honours the `NO_COLOR` environment variable) |
| `--ascii` | flag | `false` |  |  | Use only ASCII characters in human-readable output |
| `--insecure-allow-http` | flag | `false` |  |  | Allow a plain http:// base URL to a host other than localhost (the API key travels unencrypted) |
| `--debug-bodies` | flag | `false` |  |  | Log request and response bodies with -v; `state` may contain sensitive data |
| `-q, --quiet` | flag | `false` |  |  | Print errors only; suppress notices and warnings on stderr |
| `-v, --verbose` | count | `0` |  |  | Log to stderr; repeat for more (-v requests and retries, -vv everything) |

## Commands

| Command | What it does |
| --- | --- |
| [`jev eval`](#jev-eval) | Evaluate a full request (state + many questions) in one API call |
| [`jev noul`](#jev-noul) | Ask one yes/no question; returns the probability of yes |
| [`jev choice`](#jev-choice) | Pick one option from a set you define |
| [`jev score`](#jev-score) | Rate the state on an ordered rubric of 2 to 10 levels |
| [`jev validate`](#jev-validate) | Check a request file offline; nothing is sent or billed |
| [`jev batch run`](#jev-batch-run) | Evaluate every row of an input file, writing one result record per row |
| [`jev models list`](#jev-models-list) | List model names and aliases, with their description and release date |
| [`jev auth login`](#jev-auth-login) | Store an API key for the profile, once, after checking it against the API |
| [`jev auth status`](#jev-auth-status) | Show whether a key is configured, where it comes from, and whether the API accepts it |
| [`jev auth logout`](#jev-auth-logout) | Remove the stored API key |
| [`jev config get`](#jev-config-get) | Print the effective value of one setting, and where it comes from |
| [`jev config set`](#jev-config-set) | Store a setting in the selected profile (the `update.*` settings apply to every profile) |
| [`jev config unset`](#jev-config-unset) | Remove a setting from the selected profile, restoring its default |
| [`jev config list`](#jev-config-list) | Print every effective setting and where its value comes from (flag, env, profile, config or default) |
| [`jev config path`](#jev-config-path) | Print the path of the configuration file |
| [`jev profile list`](#jev-profile-list) | List the profiles, marking the active one |
| [`jev profile use`](#jev-profile-use) | Make a profile the one used when neither --profile nor `JEV_PROFILE` selects another |
| [`jev profile create`](#jev-profile-create) | Create a profile; --base-url, --model, --output, --timeout and --max-retries become its settings |
| [`jev profile delete`](#jev-profile-delete) | Delete a profile and its settings |
| [`jev schema request`](#jev-schema-request) | JSON Schema of a complete request: `state`, `model` and `questions`, as the API takes it |
| [`jev schema questions`](#jev-schema-questions) | JSON Schema of a question set: a request file in which only `questions` is required |
| [`jev schema batch-record`](#jev-schema-batch-record) | JSON Schema of one line of `jev batch run` output |
| [`jev schema output`](#jev-schema-output) | JSON Schema of the result that `jev eval`, `jev noul`, `jev choice` and `jev score` print |
| [`jev schema error`](#jev-schema-error) | JSON Schema of the error object printed on stderr for a program |
| [`jev spec`](#jev-spec) | Dump the entire command tree as JSON, for agents and tooling |
| [`jev mcp serve`](#jev-mcp-serve) | Serve jev's tools to an MCP client (Claude Code, Claude Desktop, Cursor, VS Code) over stdio |
| [`jev update`](#jev-update) | Update jev, check for an update, or roll one back |
| [`jev completion`](#jev-completion) | Print a shell completion script |
| [`jev version`](#jev-version) | Print the version, commit, build date and target |

### jev eval

Evaluate a full request (state + many questions) in one API call

```text
jev eval [OPTIONS] --file <FILE>
```

**When to use.** Use `jev eval` to ask several questions about the same state in one call: the answers come back together, and extra questions cost tokens, not time. Use `jev noul`, `jev choice` or `jev score` for one question written on the command line, `jev validate` to check a request file without sending it, and `jev batch run` to apply the same questions to many states.

**Input.** A request file (-f), JSON or YAML, shaped like the API body: `questions`, a map of question id to question, and optionally `state` and `model`. The state is --state (text) or --state-file (a path, or `-` for stdin), else the file's `state`, else a pipe. The model is --model, else the file's `model`, else TYPESAFE_DEFAULT_MODEL, else the profile, else `jev-latest`.

**Output.** The result envelope: `model` (the versioned id that answered), `requested_model`, `answers` (one per question id, exactly as the API returned them), `usage`, `cost_usd` (an estimate), `request_id` and `latency_ms`, plus `gate` when --assert was used. --dry-run prints the request body and a size estimate instead, and sends nothing; --raw prints the API's response body as received.

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `-f, --file <FILE>` | string, required |  |  |  | Request file, JSON or YAML, with `questions` and optionally `state` and `model`; `-` reads stdin |
| `--input-format <FORMAT>` | `json`, `yaml` |  |  |  | Format of the request file |

_Gating_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--assert <CONDITION>` | string, repeatable |  |  |  | Exit 10 unless this holds, e.g. `is_urgent >= 0.7`, `team == billing`, `team.confidence >= 0.8` |

_State_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--state <TEXT>` | string |  |  |  | The state to evaluate, as text (`-` is not stdin here: use --state-file -) |
| `--state-file <PATH>` | string |  |  |  | Read the state from a file, or from stdin with `-` |
| `--state-format <FORMAT>` | `auto`, `text`, `json` | `auto` |  |  | How to read the state: `json` sends an object or array, `text` one string |

_Sending_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--dry-run` | flag | `false` |  |  | Validate, print the exact request body and a size estimate, and send nothing |
| `--strict` | flag | `false` |  |  | Treat validation warnings as errors |
| `--skip-size-check` | flag | `false` |  |  | Skip the offline estimate of the request's size |
| `--warn-unpinned` | flag | `false` |  |  | Remind me on stderr to pin a versioned model id when the request used an alias |
| `--raw` | flag | `false` |  |  | Print the API response body exactly as received, ignoring --output |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and every --assert condition holds |
| 2 | usage or validation error; nothing was sent |
| 3 | no API key, or the API refused it |
| 4 | the API rejected the request |
| 5 | rate limited or overloaded, after retries |
| 6 | network failure, timeout or server error, after retries |
| 10 | evaluated, and an --assert condition is false (never an error) |

**Examples**

```bash
# Several questions about one ticket, in a single call
jev eval -f triage.yaml --state-file ticket.txt

# Only one answer's probability, for a script
jev eval -f triage.yaml --state-file ticket.txt --field answers.is_urgent.noul

# Gate a CI step: exit 10 unless the change is confidently low risk
jev eval -f review.yaml --state-file diff.txt \
    --assert "risk == low" --assert "risk.confidence >= 0.8"

# Check the request and see its size, without sending it or needing a key
jev eval -f triage.yaml --state-file ticket.txt --dry-run -o json
```

### jev noul

Ask one yes/no question; returns the probability of yes

```text
jev noul [OPTIONS] [QUESTION]
```

**When to use.** Use `jev noul` when the answer is yes or no about one property: does this hold? The probability is absolute: it can be low even when nothing fits better, and several nouls can all be high, so ask one noul per label when several labels can apply at once. Use `jev choice` when exactly one of several options must be picked (which one?), and `jev score` for a position on a described scale (how much?). A value near 0.5 means the model cannot tell, not "medium".

**Input.** QUESTION in plain words, or --instructions-file with structured instructions. --true and --false (or --criteria-file) say what yes and no mean. The state is --state (text) or --state-file (a path, or `-` for stdin), else a pipe. Jev reads literally and cannot count, do arithmetic or compare dates: keep those in code.

**Output.** `noul`, the probability of yes from 0 to 1, then `model`, `requested_model`, `usage`, `cost_usd` (an estimate), `request_id`, `latency_ms`, and `gate` when a gating flag was used. `--field noul` prints just the number.

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `QUESTION` | string | The question, in plain words |

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--instructions-file <FILE>` | string |  |  |  | Structured instructions (JSON or YAML: `what`, `not_for`, `examples`, ...) instead of QUESTION |
| `--true <TEXT>` | string |  |  |  | What a yes (a value near 1) means |
| `--false <TEXT>` | string |  |  |  | What a no (a value near 0) means |
| `--criteria-file <FILE>` | string |  |  |  | Structured criteria (JSON or YAML) with `true` and `false` |

_Gating_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--fail-under <P>` | number |  |  |  | Exit 10 when P(yes) is below this |
| `--fail-over <P>` | number |  |  |  | Exit 10 when P(yes) is above this |
| `--abstain-band <LO,HI>` | string |  |  |  | Exit 11 when P(yes) is inside `LO,HI`, e.g. `0.4,0.6`: near 0.5 the model cannot tell |

_State_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--state <TEXT>` | string |  |  |  | The state to evaluate, as text (`-` is not stdin here: use --state-file -) |
| `--state-file <PATH>` | string |  |  |  | Read the state from a file, or from stdin with `-` |
| `--state-format <FORMAT>` | `auto`, `text`, `json` | `auto` |  |  | How to read the state: `json` sends an object or array, `text` one string |

_Sending_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--dry-run` | flag | `false` |  |  | Validate, print the exact request body and a size estimate, and send nothing |
| `--strict` | flag | `false` |  |  | Treat validation warnings as errors |
| `--skip-size-check` | flag | `false` |  |  | Skip the offline estimate of the request's size |
| `--warn-unpinned` | flag | `false` |  |  | Remind me on stderr to pin a versioned model id when the request used an alias |
| `--raw` | flag | `false` |  |  | Print the API response body exactly as received, ignoring --output |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and the condition holds (or no gate was given) |
| 2 | usage or validation error; nothing was sent |
| 3 | no API key, or the API refused it |
| 4 | the API rejected the request |
| 5 | rate limited or overloaded, after retries |
| 6 | network failure, timeout or server error, after retries |
| 10 | evaluated, and P(yes) is below --fail-under or above --fail-over (never an error) |
| 11 | evaluated, and P(yes) is inside --abstain-band: the model cannot tell |

**Examples**

```bash
# Branch in a shell script: exit 0 when P(yes) >= 0.7, exit 10 when it is lower
if git log -1 --pretty=%B | jev noul "Does this commit describe a user-facing change?" \
     --fail-under 0.7; then
  echo "needs a changelog entry"
fi

# Just the probability
jev noul "Is the customer angry?" --state-file ticket.txt --field noul

# Route uncertain answers to a person: exit 11 when the model cannot tell
jev noul "Is this spam?" --state "$MESSAGE" --fail-under 0.6 --abstain-band 0.4,0.6
```

### jev choice

Pick one option from a set you define

```text
jev choice [OPTIONS] [QUESTION]
```

**When to use.** Use `jev choice` when exactly one of several options is the answer: which one? The probabilities are relative and add up to 1, so always offer a way out such as `other` or `not_stated`: the model cannot pick an option you left out. Use `jev noul` when the answer is yes or no, or when several labels can apply at once (one noul per label), and `jev score` when the options are ordered levels of one scale. Two options that mean yes and no are a noul.

**Input.** QUESTION in plain words, or --instructions-file. The options are repeated --option `name` or `name=description` (up to 255), or --criteria-file with a map of name to description; describe options that are easy to confuse. The state is --state (text) or --state-file (a path, or `-` for stdin), else a pipe.

**Output.** `choice` (the winning option), `confidence` (how peaked the distribution is, not how likely it is to be right) and `probabilities` per option, then `model`, `requested_model`, `usage`, `cost_usd` (an estimate), `request_id`, `latency_ms`, and `gate` when a gating flag was used. `--field choice` prints just the winner.

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `QUESTION` | string | The question, in plain words |

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--instructions-file <FILE>` | string |  |  |  | Structured instructions (JSON or YAML: `what`, `not_for`, `examples`, ...) instead of QUESTION |
| `--option <NAME[=DESCRIPTION]>` | string, repeatable |  |  |  | An option, as `name` or `name=description`; repeat for each (up to 255). Include a way out such as `other` |
| `--criteria-file <FILE>` | string |  |  |  | Structured criteria (JSON or YAML): a map of option name to description |
| `--no-escape-warning` | flag | `false` |  |  | Do not warn when no option offers a way out (`other`, `none_of_the_above`, `not_stated`, ...) |

_Gating_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--expect <OPTION>` | string, repeatable |  |  |  | Exit 10 unless this option wins; repeat to accept any of several |
| `--min-confidence <C>` | number |  |  |  | Exit 10 when the confidence is below this |

_State_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--state <TEXT>` | string |  |  |  | The state to evaluate, as text (`-` is not stdin here: use --state-file -) |
| `--state-file <PATH>` | string |  |  |  | Read the state from a file, or from stdin with `-` |
| `--state-format <FORMAT>` | `auto`, `text`, `json` | `auto` |  |  | How to read the state: `json` sends an object or array, `text` one string |

_Sending_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--dry-run` | flag | `false` |  |  | Validate, print the exact request body and a size estimate, and send nothing |
| `--strict` | flag | `false` |  |  | Treat validation warnings as errors |
| `--skip-size-check` | flag | `false` |  |  | Skip the offline estimate of the request's size |
| `--warn-unpinned` | flag | `false` |  |  | Remind me on stderr to pin a versioned model id when the request used an alias |
| `--raw` | flag | `false` |  |  | Print the API response body exactly as received, ignoring --output |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and the condition holds (or no gate was given) |
| 2 | usage or validation error; nothing was sent |
| 3 | no API key, or the API refused it |
| 4 | the API rejected the request |
| 5 | rate limited or overloaded, after retries |
| 6 | network failure, timeout or server error, after retries |
| 10 | evaluated, and the winner is not --expect or is below --min-confidence (never an error) |

**Examples**

```bash
# Pick a team, always offering a way out, and print only the winner
jev choice "Which team should handle this?" --state-file ticket.txt \
    --option billing="Payments, invoices, refunds" --option technical="Bugs, outages" \
    --option other --field choice

# Branch in a shell script: exit 0 when either option wins with enough confidence, 10 otherwise
if jev choice "Which team?" --state-file ticket.txt \
     --option billing --option sales --option other \
     --expect billing --expect sales --min-confidence 0.8; then
  echo "route to revenue"
fi
```

### jev score

Rate the state on an ordered rubric of 2 to 10 levels

```text
jev score [OPTIONS] [QUESTION]
```

**When to use.** Use `jev score` to place the state on one ordered scale: how much, how severe, how good? Each level is judged on its own, so describe a situation per level rather than "worse than the previous one", and rate one dimension per score. Use `jev noul` for yes or no, and `jev choice` for options with no order.

**Input.** QUESTION in plain words, or --instructions-file. The levels are repeated --level, lowest first (2 to 10), or --criteria-file with an ordered list; they are numbered from 0. The state is --state (text) or --state-file (a path, or `-` for stdin), else a pipe.

**Output.** `score` (the expected level, from 0 to the highest level), `confidence`, `probabilities` and `legend` per level, then `model`, `requested_model`, `usage`, `cost_usd` (an estimate), `request_id`, `latency_ms`, and `gate` when a gating flag was used. `--field score` prints just the number.

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `QUESTION` | string | The question, in plain words |

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--instructions-file <FILE>` | string |  |  |  | Structured instructions (JSON or YAML: `what`, `not_for`, `examples`, ...) instead of QUESTION |
| `--level <DESCRIPTION>` | string, repeatable |  |  |  | A level of the rubric, lowest first; repeat for each (2 to 10) |
| `--criteria-file <FILE>` | string |  |  |  | Structured criteria (JSON or YAML): an ordered list of level descriptions |

_Gating_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--fail-under <SCORE>` | number |  |  |  | Exit 10 when the score is below this |
| `--fail-over <SCORE>` | number |  |  |  | Exit 10 when the score is above this |
| `--min-confidence <C>` | number |  |  |  | Exit 10 when the confidence is below this |

_State_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--state <TEXT>` | string |  |  |  | The state to evaluate, as text (`-` is not stdin here: use --state-file -) |
| `--state-file <PATH>` | string |  |  |  | Read the state from a file, or from stdin with `-` |
| `--state-format <FORMAT>` | `auto`, `text`, `json` | `auto` |  |  | How to read the state: `json` sends an object or array, `text` one string |

_Sending_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--dry-run` | flag | `false` |  |  | Validate, print the exact request body and a size estimate, and send nothing |
| `--strict` | flag | `false` |  |  | Treat validation warnings as errors |
| `--skip-size-check` | flag | `false` |  |  | Skip the offline estimate of the request's size |
| `--warn-unpinned` | flag | `false` |  |  | Remind me on stderr to pin a versioned model id when the request used an alias |
| `--raw` | flag | `false` |  |  | Print the API response body exactly as received, ignoring --output |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and the condition holds (or no gate was given) |
| 2 | usage or validation error; nothing was sent |
| 3 | no API key, or the API refused it |
| 4 | the API rejected the request |
| 5 | rate limited or overloaded, after retries |
| 6 | network failure, timeout or server error, after retries |
| 10 | evaluated, and --fail-under, --fail-over or --min-confidence does not hold (never an error) |

**Examples**

```bash
# Rate on a rubric; levels are numbered from 0 in the order given
jev score "How frustrated is the customer?" --state-file ticket.txt \
    --level Calm --level Frustrated --level "Very angry"

# Branch in a shell script: exit 10 when the score is above 1.5
if ! jev score "How risky is this change?" --state-file diff.txt \
     --level Safe --level Risky --level Dangerous --fail-over 1.5 --field score; then
  echo "needs a second reviewer"
fi
```

### jev validate

Check a request file offline; nothing is sent or billed

```text
jev validate [OPTIONS] --file <FILE>
```

**When to use.** Use `jev validate` while writing a request file, in CI, or before a large batch: it runs every check `jev eval` runs before sending, offline, with no API key and no cost. Use `jev eval --dry-run` to also see the exact request body that would be sent.

**Input.** A request file (-f), JSON or YAML; `-` reads stdin. `state` and `model` may be left out. --state or --state-file adds a state, so that the size of the request can be estimated. Nothing is read from a pipe unless asked for with `-`.

**Output.** `file`, `valid`, `strict`, `summary` (counts of `errors` and `warnings`), `findings` (each with `severity`, `rule`, `question`, `path` as a JSON Pointer, `message` and `suggestion`) and `size`, the estimated input tokens when there is a state.

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `-f, --file <FILE>` | string, required |  |  |  | Request file, JSON or YAML; `state` and `model` may be left out; `-` reads stdin |
| `--input-format <FORMAT>` | `json`, `yaml` |  |  |  | Format of the request file |
| `--state <TEXT>` | string |  |  |  | Check the request with this state, as text, so that its size can be estimated (`-` is not stdin here: use --state-file -) |
| `--state-file <PATH>` | string |  |  |  | Check the request with the state in this file, or on stdin with `-` |
| `--state-format <FORMAT>` | `auto`, `text`, `json` | `auto` |  |  | How to read a state given outside the request file |
| `--strict` | flag | `false` |  |  | Count warnings as errors, so that any finding makes the request invalid |
| `--skip-size-check` | flag | `false` |  |  | Skip the offline estimate of the request's size |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | the request is valid; warnings allowed unless --strict |
| 2 | the request is not valid, or the command line is wrong |

**Examples**

```bash
# Check a request file while writing it
jev validate -f triage.yaml

# Fail CI on any finding, warnings included, with a state for the size check
jev validate -f triage.yaml --state-file ticket.txt --strict -o json

# List the rules a file breaks
jev validate -f triage.yaml -o json | jq -r '.findings[].rule'
```

### jev batch run

Evaluate every row of an input file, writing one result record per row

```text
jev batch run [OPTIONS] --file <FILE>
```

**When to use.** Use `jev batch run` to ask the same questions about many states: tickets, messages, records. It sends one request per row, a few at a time, and writes one result record per row. Use `jev eval` for one state and several questions, `jev noul`, `jev choice` or `jev score` for one state and one question, and `jev validate` to check the question set without sending anything. Run it with --dry-run first to see how many requests a run makes and what it should cost.

**Input.** A request file (-f), JSON or YAML, whose `questions` (and `model`, if present) are used for every row; any `state` in it is replaced by each row's. The rows are --input, a JSONL file (one JSON value per line), a CSV file with a header row or a JSON file holding one array whose elements are the rows (--input-format json, at most 50 MB; a `.json` file holding one array is read so without it), or JSONL piped on stdin; --limit reads only the first N. A row's state is the whole row, one field of it (--state-field) or an object of some fields (--state-fields); its id is --id-field, else its line number (in an array, its 1-based index). Before anything is sent the question set is validated and every row of a file is checked: a malformed row, a missing field or a repeated id is exit 2. Piped rows can be read only once, so they are checked as they arrive, and such a row stops sending there. --resume continues into an existing --out file: rows it records `ok` are not sent again, rows it records `error` are, and an incomplete last line left by a crash is removed first. A 429 or 529 on any request pauses every worker for the server's `retry-after` and then spaces requests out until they succeed again. SIGINT or SIGTERM stops sending, gives the requests in flight up to 10 s to finish and be recorded, and exits 130; a second signal exits at once.

**Output.** JSON Lines on stdout, or appended to --out: one record per row, in the order rows finish (--ordered: in input order), whatever --output says. Each record is written whole with its newline, so the output is valid JSONL even after an interruption. An answered row is `{id, status: "ok", model, answers, usage, cost_usd, request_id, latency_ms}`; a failed one is `{id, status: "error", error}`, where `error` has the shape of every JSON error (`code`, `exit_code`, `error_type`, `message`, `hint`, `request_id`, ...). A failed row does not stop the run unless --fail-fast or --max-errors says so. The summary goes to stderr (one `{"summary": ...}` JSON line when the output is for a program) and to --summary-json: `rows_total`, `ok`, `failed`, `skipped`, `already_ok` (not sent: --resume found them answered), `input_tokens`, `cost_usd` (an estimate), `wall_time_ms`, `rows_per_second`, `retries`, `models` and `stopped_by` (`fail_fast`, `max_errors`, `invalid_input`, `output_closed`, `interrupted` or null). On a terminal, stderr shows a progress bar; elsewhere it gets a line every 5 s once a run has taken that long (`{"progress": ...}` for a program), and --quiet silences both. --dry-run sends nothing and needs no key: stdout gets `{dry_run, rows_total, requests, already_ok, invalid_rows, estimated_input_tokens, estimated_cost_usd, requested_model, problems}`; the cost is null for an alias such as `jev-latest`, which has no price.

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `-f, --file <FILE>` | string, required |  |  |  | Question set: a request file (JSON or YAML) with `questions` and optionally `model` |

_Rows_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--input <PATH>` | string |  |  |  | Rows to evaluate: a JSONL, CSV or JSON array file, or `-` for JSONL on stdin |
| `--input-format <FORMAT>` | `jsonl`, `csv`, `json` |  |  |  | Format of --input |
| `--state-field <NAME>` | string |  |  |  | Send this one field of each row as its state |
| `--state-fields <NAME,...>` | string, repeatable |  |  |  | Send an object of only these fields as the state, e.g. `subject,body` |
| `--id-field <NAME>` | string |  |  |  | Field that identifies each row in the results; must be unique |
| `--limit <N>` | integer |  |  |  | Read only the first N rows of the input |

_Results_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--out <PATH>` | string |  |  |  | Write the records to this file instead of stdout; it must not exist yet, or be empty, unless --resume |
| `--resume` | flag | `false` |  |  | Continue into --out: skip rows it records `ok`, send the rest again, append their records |
| `--ordered` | flag | `false` |  |  | Write the records in input order instead of as rows finish; slower when one row is slow |
| `--summary-json <PATH>` | string |  |  |  | Also write the end-of-run summary to this file, as one JSON object |

_Sending_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--dry-run` | flag | `false` |  |  | Check every row and print the number of requests and their estimated cost; send nothing |

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--concurrency <N>` | integer | `4` |  | `concurrency` | Requests in flight at once, 1 to 64; shared keys get rate limited above about 8 |
| `--fail-fast` | flag | `false` |  |  | Stop sending at the first row that fails; rows in flight still finish |
| `--max-errors <N>` | integer |  |  |  | Stop sending once N rows have failed; rows in flight still finish |

_Sending_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--strict` | flag | `false` |  |  | Treat validation warnings as errors |
| `--skip-size-check` | flag | `false` |  |  | Skip the offline estimate of each request's size |
| `--warn-unpinned` | flag | `false` |  |  | Remind me on stderr to pin a versioned model id when the run used an alias |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | every row was answered, or a dry run found every row usable |
| 2 | usage error, or a bad question set or input row; nothing was sent for a file |
| 3 | no API key, or the API refused it |
| 7 | the run finished, but some rows failed: see their error records, then --resume |
| 130 | interrupted by SIGINT or SIGTERM; rerun with --resume to finish |

**Examples**

```bash
# Label tickets: the `body` field is the state, records keyed by ticket_id
jev batch run -f triage.yaml --input tickets.jsonl --state-field body \
    --id-field ticket_id --out results.jsonl

# See how many requests the run makes and its estimated cost, sending nothing
jev batch run -f triage.yaml --input tickets.jsonl --state-field body \
    --model jev-1.13.0 --dry-run

# Continue an interrupted or partly failed run: only unanswered rows are sent
jev batch run -f triage.yaml --input export.csv --state-fields subject,body \
    --out results.jsonl --resume --summary-json summary.json

# Rows from a pipeline, in input order, stopping at the first failure
jq -c '.items[]' dump.json | jev batch run -f triage.yaml --ordered --fail-fast \
    | jq -r 'select(.status == "error") | .id'
```

### jev models list

List model names and aliases, with their description and release date

```text
jev models list [OPTIONS]
```

**When to use.** Use `jev models list` to see the model names and aliases the account can use. A versioned id such as `jev-1.13.0` is accepted by --model even when it is not listed here. Aliases such as `jev-latest` move to newer versions without notice, so pin a versioned id once thresholds have been tuned against it. Use `jev auth status` to check the key alone, and `jev version` for the version of jev itself.

**Input.** Nothing but an API key (TYPESAFE_API_KEY or `jev auth login`). --profile and --base-url choose the account and the endpoint.

**Output.** `models`: one entry per model or alias, with `name`, `description` and `release_date`.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | usage or validation error; nothing was sent |
| 3 | no API key, or the API refused it |
| 4 | the API rejected the request |
| 5 | rate limited or overloaded, after retries |
| 6 | network failure, timeout or server error, after retries |

**Examples**

```bash
# What the account can use
jev models list

# Only the names, for a script
jev models list -o json | jq -r '.models[].name'
```

### jev auth login

Store an API key for the profile, once, after checking it against the API

```text
jev auth login [OPTIONS]
```

**When to use.** Use `jev auth login` once per machine and profile, to store an API key in jev's private credentials file. Where TYPESAFE_API_KEY is set (CI, containers, agents) there is nothing to log in to: the variable always wins. Use `jev auth status` to see which key is in use, and `jev auth logout` to remove a stored one.

**Input.** The key, typed at a prompt that does not echo, or on stdin with --with-token. The key is never accepted as a flag value, so that it cannot end up in shell history or a process list. --profile chooses the profile it is stored for; --skip-verify stores it without checking it against the API. jev looks for a key in TYPESAFE_API_KEY first, then in the credentials file this command writes (next to config.toml, readable only by you).

**Output.** `profile`, `stored_in` (the credentials file), `verified` and `fingerprint`, the last four characters of the key. Nothing more of the key is ever shown.

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--with-token` | flag | `false` |  |  | Read the key from stdin instead of prompting (for scripts and agents) |
| `--skip-verify` | flag | `false` |  |  | Store the key without checking it against the API first |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | the key is stored |
| 2 | no key was given, or a prompt was needed but not allowed: use --with-token |
| 3 | the API refused the key; nothing was stored |
| 5 | rate limited or overloaded, after retries |
| 6 | network failure, timeout or server error, after retries |

**Examples**

```bash
# Interactive: type or paste the key at a prompt that does not echo
jev auth login

# Automation: read the key from stdin
printf %s "$TYPESAFE_KEY" | jev auth login --with-token --profile ci -o json
```

### jev auth status

Show whether a key is configured, where it comes from, and whether the API accepts it

```text
jev auth status [OPTIONS]
```

**When to use.** Use `jev auth status` to find out whether jev has an API key, where it comes from (the environment or the credentials file) and whether the API accepts it. Use `jev auth login` to store a key, and `jev models list` to see what the key gives access to.

**Input.** Nothing. --profile chooses the profile; --offline skips the check against the API.

**Output.** `profile`, `authenticated`, `source` (`env` or `file`), `fingerprint` (the last four characters of the key), `base_url` and `check` (`ok`, and `detail` when it failed). The key itself is never shown.

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--offline` | flag | `false` |  |  | Do not check the key against the API |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | a key is configured, and the API accepts it (or --offline) |
| 2 | the configuration cannot be read |
| 3 | no key is configured, or the API refused it |

**Examples**

```bash
# Is there a key, and does it work?
jev auth status

# Check without the network, for a script
jev auth status --offline --field authenticated
```

### jev auth logout

Remove the stored API key

```text
jev auth logout [OPTIONS]
```

**When to use.** Use `jev auth logout` to delete a key stored by `jev auth login`, for the selected profile or, with --all, for every profile. It cannot unset TYPESAFE_API_KEY: unset the variable yourself. Use `jev auth status` to check what is left.

**Input.** Nothing. --profile chooses the profile; --all removes the keys of every profile.

**Output.** `removed`: the profiles whose stored key was deleted, empty when there was none.

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--all` | flag | `false` |  |  | Remove the stored keys of every profile, not only the selected one |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | usage or validation error; nothing was sent |

**Examples**

```bash
# Forget the key of one profile
jev auth logout --profile staging

# Forget every stored key
jev auth logout --all -o json
```

### jev config get

Print the effective value of one setting, and where it comes from

```text
jev config get [OPTIONS] <KEY>
```

**When to use.** Use `jev config get` to see the value one setting will have, and where it comes from: a flag, the environment, the profile or the default. Use `jev config list` for every setting at once, and `jev config set` to change one.

**Input.** KEY, one of `base_url`, `model`, `output`, `timeout`, `max_retries`, `concurrency` and `warn_unpinned`, which each profile has, and `update.auto`, `update.channel` and `update.pin_version`, which every profile shares. --profile reads another profile, and flags on the same command line count, as they would on a real run.

**Output.** `key`, `value`, `source` (`flag`, `env`, `profile`, `config` for an `update.*` setting in the file, or `default`), `origin` (the flag, variable, profile or key) and `description`. `--field value` prints just the value.

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `KEY` | string, required | Setting name: `base_url`, `model`, `output`, `timeout`, `max_retries`, `concurrency`, `warn_unpinned`, `update.auto`, `update.channel` or `update.pin_version` |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | not a setting, or the configuration cannot be read |

**Examples**

```bash
# Which model would a run use, and why?
jev config get model

# Just the value, for a script
jev config get timeout --field value
```

### jev config set

Store a setting in the selected profile (the `update.*` settings apply to every profile)

```text
jev config set [OPTIONS] <KEY> <VALUE>
```

**When to use.** Use `jev config set` to store a default in the selected profile, so that you do not repeat a flag, or to change how jev updates itself (`update.*`, for every profile). For one run, pass the flag or set the environment variable instead; `jev config unset` restores the default. The API key is not a setting: use `jev auth login`.

**Input.** KEY and VALUE. The value is checked before anything is written: `timeout` takes `30s` or `500ms`, `output` takes table, json, yaml or jsonl, `update.auto` true or false, `update.channel` stable or prerelease, `update.pin_version` a version such as 0.3.1 (which turns automatic updates off), and so on. --profile chooses the profile to change; the `update.*` settings go to the `[update]` table, which every profile shares.

**Output.** `profile` (`null` for an `update.*` setting), `key`, `value` and `changed`.

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `KEY` | string, required | Setting name: `base_url`, `model`, `output`, `timeout`, `max_retries`, `concurrency`, `warn_unpinned`, `update.auto`, `update.channel` or `update.pin_version` |
| `VALUE` | string, required | The new value, e.g. `jev-1.13.0`, `45s`, `json` or `false` |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | the setting is stored |
| 2 | not a setting, a value it does not accept, or the configuration cannot be written |

**Examples**

```bash
# Pin a versioned model once thresholds are tuned against it
jev config set model jev-1.13.0

# A longer timeout for the ci profile
jev config set timeout 45s --profile ci -o json

# Stop jev updating itself (JEV_AUTO_UPDATE=false does it for one environment)
jev config set update.auto false
```

### jev config unset

Remove a setting from the selected profile, restoring its default

```text
jev config unset [OPTIONS] <KEY>
```

**When to use.** Use `jev config unset` to remove a setting from the selected profile, so that the environment or the built-in default applies again. Use `jev config set` to change it instead, and `jev config get` to see the value that now applies.

**Input.** KEY, as for `jev config get`. --profile chooses the profile to change; an `update.*` setting is removed for every profile.

**Output.** `profile` (`null` for an `update.*` setting), `key`, `value` (null) and `changed`, false when it was not set.

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `KEY` | string, required | Setting name: `base_url`, `model`, `output`, `timeout`, `max_retries`, `concurrency`, `warn_unpinned`, `update.auto`, `update.channel` or `update.pin_version` |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | not a setting, or the configuration cannot be written |

**Examples**

```bash
# Back to the default timeout
jev config unset timeout

# Stop pinning a model in the ci profile
jev config unset model --profile ci -o json
```

### jev config list

Print every effective setting and where its value comes from (flag, env, profile, config or default)

```text
jev config list [OPTIONS]
```

**When to use.** Use `jev config list` to see every effective setting and where each value comes from, e.g. to find out why a run used an unexpected model. Use `jev config get` for one setting, and `jev profile list` for the profiles themselves.

**Input.** Nothing. --profile lists another profile, and flags on the same command line count.

**Output.** `profile` (the selected profile, and how it was selected), `settings` (one row per setting with `key`, `value`, `source`, `origin` and `description`, as `jev config get` describes them) and `config_file`.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | the configuration cannot be read |

**Examples**

```bash
# Every setting and where it comes from
jev config list

# Only the settings that are not defaults
jev config list -o json | jq '.settings[] | select(.source != "default")'
```

### jev config path

Print the path of the configuration file

```text
jev config path [OPTIONS]
```

**When to use.** Use `jev config path` to find the configuration file, to read or edit it by hand. It works even when the file is broken, so it is the way to find a file that needs fixing. Use `jev config list` to see the values in effect.

**Input.** Nothing. JEV_CONFIG_DIR moves the configuration directory.

**Output.** `config_dir`, `config_file` and `exists`. `--field config_file` prints just the path.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | the configuration directory cannot be worked out |

**Examples**

```bash
# Where the configuration lives
jev config path

# Only the file's path, for a script
jev config path --field config_file
```

### jev profile list

List the profiles, marking the active one

```text
jev profile list [OPTIONS]
```

**When to use.** Use `jev profile list` to see the profiles, which one is active and what each one sets. Use `jev config list` for the values a run would actually use, after flags and environment variables.

**Input.** Nothing.

**Output.** `profiles`: one entry per profile, with `name`, `active` (used when nothing selects another), `selected` (used by this run) and `settings`.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | the configuration cannot be read |

**Examples**

```bash
# The profiles, marking the active one
jev profile list

# Only the names, for a script
jev profile list -o json | jq -r '.profiles[].name'
```

### jev profile use

Make a profile the one used when neither --profile nor `JEV_PROFILE` selects another

```text
jev profile use [OPTIONS] <NAME>
```

**When to use.** Use `jev profile use` to change the profile that applies when neither --profile nor JEV_PROFILE selects one. For a single run, pass --profile instead. Use `jev profile create` first when the profile does not exist.

**Input.** NAME, an existing profile.

**Output.** `profile` and `action`, which is `active`.

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `NAME` | string, required | Name of an existing profile |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | no such profile, or the configuration cannot be written |

**Examples**

```bash
# Work against staging from now on
jev profile use staging

# Back to the default profile
jev profile use default -o json
```

### jev profile create

Create a profile; --base-url, --model, --output, --timeout and --max-retries become its settings

```text
jev profile create [OPTIONS] <NAME>
```

**When to use.** Use `jev profile create` to keep a separate set of defaults, e.g. for a staging endpoint or for CI. Use `jev config set --profile NAME` to change one setting of an existing profile, and `jev auth login --profile NAME` to give it a key.

**Input.** NAME: letters, digits, `-` and `_`. --base-url, --model, --output, --timeout and --max-retries given on the same command line become its settings.

**Output.** `profile` and `action`, which is `created`.

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `NAME` | string, required | Name for the new profile: letters, digits, `-` and `_` |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | the name is taken or not valid, or the configuration cannot be written |

**Examples**

```bash
# A profile for a staging endpoint with a pinned model
jev profile create staging --base-url https://staging.example.com --model jev-1.13.0

# A profile with a longer timeout
jev profile create ci --timeout 60s -o json
```

### jev profile delete

Delete a profile and its settings

```text
jev profile delete [OPTIONS] <NAME>
```

**When to use.** Use `jev profile delete` to remove a profile, its settings and its stored API key. Use `jev config unset` to remove one setting instead, and `jev auth logout` to remove only the key.

**Input.** NAME, an existing profile other than `default`.

**Output.** `profile` and `action`, which is `deleted`.

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `NAME` | string, required | Name of the profile to delete |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | no such profile, or the configuration cannot be written |

**Examples**

```bash
# Remove a profile that is no longer needed
jev profile delete staging

# The same, for a script
jev profile delete staging -o json
```

### jev schema request

JSON Schema of a complete request: `state`, `model` and `questions`, as the API takes it

```text
jev schema request [OPTIONS]
```

**When to use.** Use `jev schema request` to author or check a file that carries everything an evaluation needs: `state`, `model` and `questions`, exactly the body the API takes. Use `jev schema questions` for a file whose state comes from elsewhere, and `jev validate` to check a file offline with jev's own rules, which explain each problem and suggest the fix.

**Input.** Nothing. It needs no API key and no configuration, and never uses the network.

**Output.** A JSON Schema (draft 2020-12) with every definition inlined, generated from the types jev reads. It holds the limits: 1 to 255 choice options, 2 to 10 score levels, at least one question, and no unknown fields. JSON on a terminal too; -o yaml gives YAML.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | usage or validation error; nothing was sent |

**Examples**

```bash
# Save it for an editor or a JSON Schema validator
jev schema request -o json > jev-request.schema.json

# The fields a question may have
jev schema request | jq '.properties.questions.additionalProperties'
```

### jev schema questions

JSON Schema of a question set: a request file in which only `questions` is required

```text
jev schema questions [OPTIONS]
```

**When to use.** Use `jev schema questions` to author or check a question set: a request file in which only `questions` is required, as given to `jev eval -f` with --state-file, to `jev validate` and to `jev batch run -f`. Use `jev schema request` for a file that must carry its own `state` and `model`.

**Input.** Nothing. It needs no API key and no configuration, and never uses the network.

**Output.** A JSON Schema (draft 2020-12) with every definition inlined: the request schema with only `questions` required. JSON on a terminal too; -o yaml gives YAML.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | usage or validation error; nothing was sent |

**Examples**

```bash
# Save it for an editor, then check a file offline
jev schema questions -o json > questions.schema.json
jev validate -f triage.yaml

# The question types
jev schema questions \
    | jq '.properties.questions.additionalProperties.oneOf[].properties.type'
```

### jev schema batch-record

JSON Schema of one line of `jev batch run` output

```text
jev schema batch-record [OPTIONS]
```

**When to use.** Use `jev schema batch-record` before writing code that reads the JSONL of `jev batch run`: every line is one record. Use `jev schema output` for the result of a single evaluation, and `jev schema questions` for the question set a batch reads.

**Input.** Nothing. It needs no API key and no configuration, and never uses the network.

**Output.** A JSON Schema (draft 2020-12) of one record: `anyOf` an `ok` record (`id`, `status`, `model`, `answers`, `usage`, `cost_usd`, `request_id`, `latency_ms`) and an `error` record (`id`, `status`, and an `error` object shaped like the JSON error on stderr). JSON on a terminal too; -o yaml gives YAML.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | usage or validation error; nothing was sent |

**Examples**

```bash
# Save it to check a results file
jev schema batch-record -o json > jev-batch-record.schema.json

# The fields of a failed row
jev schema batch-record | jq '.anyOf[1].properties | keys'
```

### jev schema output

JSON Schema of the result that `jev eval`, `jev noul`, `jev choice` and `jev score` print

```text
jev schema output [OPTIONS]
```

**When to use.** Use `jev schema output` before writing code that reads the JSON of `jev eval`, `jev noul`, `jev choice` or `jev score`. Use `jev schema error` for what they print on stderr when they fail.

**Input.** Nothing. It needs no API key and no configuration, and never uses the network.

**Output.** A JSON Schema (draft 2020-12) of the result: `anyOf` the `jev eval` form, with every answer under `answers`, and the one-question form, with the answer's fields at the top level. Both have `model`, `requested_model`, `usage`, `cost_usd`, `request_id` and `latency_ms`, and `gate` when a gating flag was used. JSON on a terminal too; -o yaml gives YAML.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | usage or validation error; nothing was sent |

**Examples**

```bash
# The fields every result has
jev schema output | jq '.anyOf[0].required'

# Save it to check results in a test suite
jev schema output -o json > jev-result.schema.json
```

### jev schema error

JSON Schema of the error object printed on stderr for a program

```text
jev schema error [OPTIONS]
```

**When to use.** Use `jev schema error` before writing code that handles jev's failures: branch on `code` or `exit_code`, show `message` and follow `hint`. Use `jev schema output` for what a successful command prints on stdout.

**Input.** Nothing. It needs no API key and no configuration, and never uses the network.

**Output.** A JSON Schema (draft 2020-12) of the one JSON object a failing command prints on stderr when its output format is machine-readable, and that an MCP tool error carries: `{error: {code, exit_code, error_type, message, hint, request_id, http_status, retryable, details}}`, every field always present. JSON on a terminal too; -o yaml gives YAML.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | usage or validation error; nothing was sent |

**Examples**

```bash
# The error fields
jev schema error | jq '.properties.error.required'

# As YAML
jev schema error -o yaml
```

### jev spec

Dump the entire command tree as JSON, for agents and tooling

```text
jev spec [OPTIONS]
```

**When to use.** Use `jev spec` when a program or an AI agent needs to know what jev can do: every command, argument and flag (with its type, default, allowed values and environment variable), exit code and example, in one JSON document generated from the same definitions as the help. Use `jev --help` or `jev <command> --help` to read about commands as a person.

**Input.** Nothing. It needs no API key and no configuration, and never uses the network.

**Output.** One JSON document: `spec_version`, `name`, `version`, `about`, `global_flags`, `exit_codes` (the whole contract), `examples` and `commands`. Each command has `path`, `usage`, `about`, `implemented`, `when_to_use`, `input`, `output`, `arguments`, `flags`, `exit_codes` and `examples`. It is JSON on a terminal too; -o yaml gives YAML.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |

**Examples**

```bash
# Every command
jev spec | jq -r '.commands[].path'

# The flags of one command
jev spec | jq '.commands[] | select(.path == "noul") | .flags'
```

### jev mcp serve

Serve jev's tools to an MCP client (Claude Code, Claude Desktop, Cursor, VS Code) over stdio

```text
jev mcp serve [OPTIONS]
```

**When to use.** Use `jev mcp serve` when an AI agent should call Jev as a tool, through an MCP client such as Claude Code, Claude Desktop, Cursor or VS Code. From a shell script or CI, call `jev eval`, `jev noul`, `jev choice` or `jev score` directly instead.

**Input.** The Model Context Protocol on stdin, one JSON-RPC message per line, until the client closes stdin. Tools: evaluate (a state and many questions), noul, choice, score (one question each), validate (offline, needs no key) and list_models. With one or more --allow-dir, also batch_run: `jev batch run` over a JSONL, CSV or JSON array file, writing its records to a new file; every path must resolve inside an --allow-dir (`..` and symbolic links included), and a run over --max-batch-rows or --max-batch-cost-usd is refused before anything is sent. Without --allow-dir no tool touches a file. The API key, base URL and model come from the usual places (--profile, the environment, the profile), read once at start-up; the key is never part of a tool's input, result or error. Every call is validated offline before anything is sent.

**Output.** Protocol messages only on stdout; logs, notices and warnings go to stderr. A tool result is the same JSON envelope as `jev eval -o json`, plus `session` (calls made and estimated spend so far); batch_run's result is the run's summary, as `jev batch run` reports it, with `out` and `session`, and it sends progress notifications when the call has a progressToken. A failure is a tool error carrying the JSON error object `jev` prints on stderr, and an invalid request is a tool error listing every finding. API failures never end the server; if stdout cannot be written, it exits 1.

**Flags**

_Guardrails_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--max-cost-usd-per-call <USD>` | number |  |  |  | Refuse, before sending, any call whose estimated cost in US dollars is above this |

_Batch_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--allow-dir <DIR>` | string, repeatable |  |  |  | Offer the `batch_run` tool, reading and writing files only inside this directory; repeat for more |
| `--max-batch-rows <N>` | integer |  |  |  | Refuse, before sending, any `batch_run` over more rows than this |
| `--max-batch-cost-usd <USD>` | number |  |  |  | Refuse, before sending, any `batch_run` whose estimated cost in US dollars is above this |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | the client closed stdin |
| 2 | usage, a broken configuration, or an --allow-dir that is not a directory |

**Examples**

```bash
# Register with Claude Code
claude mcp add jev -- jev mcp serve

# Refuse any call estimated above a tenth of a cent (pin a versioned model so it has a price)
jev mcp serve --model jev-1.13.0 --max-cost-usd-per-call 0.001

# Also offer batch_run over files in one directory, at most 1000 rows and $0.05 a run
jev mcp serve --model jev-1.13.0 --allow-dir ./data \
  --max-batch-rows 1000 --max-batch-cost-usd 0.05

# Check it from a shell: list the tools' names
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | jev mcp serve \
  | jq -r '.result.tools[].name'
```

### jev update

Update jev, check for an update, or roll one back

```text
jev update [OPTIONS]
```

**When to use.** Use `jev update` to move to the latest release now, `jev update --check` in a script to learn whether one exists (exit 20) without changing anything, `jev update --rollback` to go back to the binary the last update replaced, and `jev update --version <x.y.z>` to install one particular release, which is the only way to move to an older one. When Homebrew or cargo installed jev, it prints their command instead and changes nothing. An install made by the install script also updates itself: at most once a day, after a command has finished, a detached process stages a newer release, and the next command swaps it in and says so in one line on stderr; the command's own output and exit code never change. It is off for Homebrew and cargo installs, with CI=true, JEV_AUTO_UPDATE=false, `update.auto = false` or `update.pin_version`, and when jev cannot write to its directory. Use `jev version` to see the running version, how it was installed and whether it updates itself, and why not.

**Input.** Nothing but the flags; no API key. `update.channel` (stable or prerelease) chooses the releases it follows. It contacts only GitHub Releases of shaharia-lab/jev-cli; --rollback, and an update of a package-manager install, contact nothing. Every download is checked against the release keys compiled into jev (a minisign signature of the archive and of SHA256SUMS, each naming its file and version, and the archive's SHA-256) before anything is written. The new binary is swapped in atomically next to the old one, which is kept for --rollback, and must pass a self-test (`jev version`) or the old one is put back.

**Output.** One object: `status` (`up_to_date`, `update_available`, `updated`, `rolled_back` or `managed`), `current_version` (the version that was running), `version` (the release concerned: the latest, the one installed or the one restored; `null` when nothing is published yet), `install_method` (`self_managed`, `homebrew`, `cargo` or `unknown`), `path` (the binary), `release_url` (its changelog) and `command` (what to run next, such as the package manager's command). A download that fails verification, or a new binary that fails its self-test and is rolled back, is exit 1 with the code `update_verification_failed` or `update_self_test_failed`; the installed binary is unchanged in both cases.

**Flags**

_Options_

| Flag | Value | Default | Environment | Setting | What it does |
| --- | --- | --- | --- | --- | --- |
| `--check` | flag | `false` |  |  | Only report whether a newer version exists: exit 0 when current, 20 when one is available |
| `--rollback` | flag | `false` |  |  | Put back the binary the last update replaced |
| `--version <X.Y.Z>` | string |  |  |  | Install this release instead of the latest; the only way to move to an older version |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | done, or nothing to do; --check: this is the latest version |
| 2 | bad flag, no such release, nothing to roll back, or jev's directory is read-only |
| 5 | GitHub is limiting requests; try later |
| 6 | GitHub could not be reached, or a download was cut short |
| 20 | --check: a newer version is available |

**Examples**

```bash
# Update to the latest release
jev update

# In a script: is there a newer version? (exit 20 when there is)
jev update --check -o json

# Undo the last update
jev update --rollback

# Install one particular release, older or newer
jev update --version 1.2.3
```

### jev completion

Print a shell completion script

```text
jev completion [OPTIONS] <SHELL>
```

**When to use.** Use `jev completion` once, when installing jev, to make the shell complete its commands, flags and flag values on Tab. The script is generated from the same definitions as the help, so re-run it after upgrading jev. It is for people at a shell: a program or an AI agent that needs the command tree should read `jev spec` instead.

**Input.** SHELL: bash, zsh, fish or powershell. Nothing else is read: no API key, no configuration, no network.

**Output.** The completion script, printed as is on stdout whatever --output says. Load it from the shell's start-up file or save it where the shell looks for completions (see the examples).

**Arguments**

| Argument | Value | What it is |
| --- | --- | --- |
| `SHELL` | enum, required | The shell to print the script for |

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |
| 2 | no shell, or one jev has no script for |

**Examples**

```bash
# Bash: add this line to ~/.bashrc
source <(jev completion bash)

# Zsh: add this line to ~/.zshrc, after compinit
source <(jev completion zsh)

# Fish: save it where fish loads completions from
jev completion fish > ~/.config/fish/completions/jev.fish

# PowerShell: add this line to $PROFILE
jev completion powershell | Out-String | Invoke-Expression
```

### jev version

Print the version, commit, build date and target

```text
jev version [OPTIONS]
```

**When to use.** Use `jev version` to report the jev you run, e.g. in a bug report. Use `jev --version` for the bare version, and `jev models list` for the model versions the API offers.

**Input.** Nothing.

**Output.** `version`, `commit`, `build_date`, `target`, `client_version`, `install_method` (`self_managed` for the install script, `homebrew`, `cargo` or `unknown`), `update_channel` (`stable` or `prerelease`, from `update.channel`) and `auto_update`: `enabled`, and the `reason` when it is off, naming the first guard that applies (a package manager, a configuration that cannot be read, JEV_AUTO_UPDATE or `update.auto`, `update.pin_version`, CI=true, an install not made by the install script, or a directory jev cannot write to). `--field version` prints just the version.

**Exit codes**

| Code | Meaning |
| ---: | --- |
| 0 | success, and any gate condition holds |

**Examples**

```bash
# Everything about this build
jev version

# Just the version, for a script
jev version --field version

# Does this jev update itself, and if not, why not?
jev version -o json | jq .auto_update
```
