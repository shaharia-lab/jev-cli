# Quick start

About a minute, and a fraction of a cent. You need `jev`
([installation](installation.md)) and an API key.

## 1. Give `jev` a key

Create a key in the [TypeSafe console](https://console.typesafe.ai/keys). Then either export it,
which is what CI jobs, containers and agents should do:

```bash
export TYPESAFE_API_KEY=...
```

or store it once in a private file, so you do not have to think about it again:

```bash
jev auth login
jev auth status
```

`jev auth login` prompts without echoing and writes the key with mode `0600` in a `0700`
directory. The key is never accepted as a flag value, so it cannot end up in your shell history.
See [Configuration and profiles](configuration.md#the-api-key).

## 2. Ask a yes/no question

```bash
jev noul "Is this message angry?" --state "You charged me twice. Fix it now."
```

```text
answer  noul  yes  0.91  ██████████████████░░
model jev-1.13.0 (requested jev-latest) · 42 input tokens · est. cost $0.000002 · 1 ms · request req_demo
```

`0.91` is the probability that the answer is yes. Piped or redirected, the same command prints
JSON instead of this table, and `--field noul` prints `0.91` and nothing else.

## 3. Use the answer in a script

A gating flag turns the answer into an exit code: 0 when the condition holds, 10 when it does
not, and anything else only when the call itself failed.

```bash
if jev noul "Is this message angry?" --state-file message.txt --fail-under 0.7 --quiet; then
  echo "escalate"
fi
```

That is the whole idea behind `jev`: a semantic `if` statement your shell already knows how to
use. [Scripting and CI](scripting-and-ci.md) covers the rest.

## 4. Ask several questions at once

One state, several questions, one call, one price for the state. Write `triage.yaml`:

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

`jev validate` is free and needs no key. `jev eval --dry-run` goes one step further and prints
the exact request body and what it should cost, still without sending anything.

To have your editor check request files as you type, point its YAML language server at the
published schema:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/schemas/questions.schema.json
```

## 5. Pin a model before you rely on it

`jev-latest` is an alias and moves to new model versions without notice, and an alias has no
known price. For anything repeatable, pin a versioned id:

```bash
jev models list
jev config set model jev-1.13.0
```

## Where to go next

| You want to | Read |
| --- | --- |
| Know which question type to ask | [The three question types](question-types.md) |
| Get better answers | [Writing good questions](writing-questions.md) |
| Branch a script or a CI job on an answer | [Scripting and CI](scripting-and-ci.md) |
| Label thousands of rows | [Many rows at once](batch.md) |
| Let an AI agent use `jev` | [AI agents and MCP](mcp.md) |
| Look up a flag | [Command reference](../commands.md) |
