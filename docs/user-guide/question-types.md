# The three question types

Jev answers three kinds of question. Picking the right one is most of the work, because the
answer's shape decides what your code can do with it.

| Type | Ask it when | You get back |
| --- | --- | --- |
| `noul` | A property either holds or it does not | The probability of yes, from 0 to 1 |
| `choice` | Exactly one of up to 255 options must win | The winner, its confidence, and a probability per option |
| `score` | A described scale of 2 to 10 levels | A position on the scale, and a confidence |

Each has a shortcut command for one question (`jev noul`, `jev choice`, `jev score`) and can be
one of many questions in a request file run with `jev eval`.

## noul: does this hold?

```bash
jev noul "Is the customer asking for a refund?" --state-file ticket.txt
```

The answer is a probability, not a label. Two things follow from that:

- **Nouls are absolute, not competing.** Several nouls about the same state can all be high or
  all be low. Ask one noul per label rather than expecting them to add up to 1.
- **0.5 means "cannot tell", not "medium".** Send those cases to a person with
  `--abstain-band 0.4,0.6`, which exits 11 instead of guessing.

Gate on it with `--fail-under` or `--fail-over`:

```bash
jev noul "Does this commit message describe a user-facing change?" --state-file message.txt --fail-under 0.7
```

## choice: which one of these?

```bash
jev choice "Which team should handle this?" --state-file ticket.txt \
  --option billing="Payments, invoicing, refunds" \
  --option technical="Bugs, outages, integrations" \
  --option other
```

The probabilities over your options add up to 1, so options compete. Two rules matter:

- **Always give a way out**, such as `other`, `none_of_the_above` or `not_stated`. The model
  cannot pick an option you did not offer, so without an escape it will pick the least wrong one.
  `jev choice` warns when there is none.
- **Describe options that are easy to confuse.** Say what each one covers and what it does not,
  in parallel terms, with examples that look like your real inputs.

Gate on it with `--expect` (repeatable) and `--min-confidence`:

```bash
jev choice "Which team should handle this?" --state-file ticket.txt \
  --option billing --option technical --option other --expect billing --min-confidence 0.6
```

## score: where on this scale?

```bash
jev score "How frustrated is the customer?" --state-file ticket.txt \
  --level Calm --level Frustrated --level "Very angry"
```

Levels are counted from 0, so those three give a score between 0 and 2. Two rules matter:

- **Describe each level as a recognisable situation.** Levels are judged on their own, so "worse
  than the level before" tells the model nothing.
- **One dimension per score.** "How urgent and how angry" is two questions.

Gate on it with `--fail-under`, `--fail-over` and `--min-confidence`.

## Reading the answer

- **`confidence` is not accuracy.** It says how peaked the distribution is. Two genuinely
  acceptable options give a low confidence, and that is not a problem.
- **Answers are not bit-for-bit repeatable.** Compare against thresholds, never for equality, and
  do not reuse a noul threshold for a choice or the other way round.
- **Keep the raw probability.** Store it and apply your policy in code, so the policy can change
  without asking the model again.
- **`cost_usd` is an estimate.** Only input tokens are billed, and an alias such as `jev-latest`
  has no published price, so the field can be `null`.

## What Jev will not do

Jev reads literally and quickly. It cannot count, do arithmetic, compare dates or look anything
up. Keep that in your code and ask Jev only for the meaning. There is a short list of habits that
make a real difference to accuracy in [Writing good questions](writing-questions.md).

## Several questions in one call

Extra questions cost tokens, not round trips, and the state is only paid for once. So when you
have more than one question about the same content, put them all in a request file and run
`jev eval` instead of calling the shortcuts several times.

```yaml
questions:
  is_urgent:
    type: noul
    instructions: Does the customer say the problem needs attention today?
    criteria:
      "true": A deadline, an outage, or money being lost right now
      "false": No time pressure is stated
  team:
    type: choice
    instructions: Which team should handle this ticket?
    criteria:
      billing: Charges, invoices, refunds, payouts
      technical: Errors, outages, integrations, bugs
      other: Anything else, or not enough information to tell
  frustration:
    type: score
    instructions: How frustrated does the customer sound?
    criteria:
      - Calm and neutral
      - Annoyed, but polite
      - Angry, or threatening to leave
```

```bash
jev eval -f triage.yaml --state-file ticket.txt -o json
```

Each answer comes back under its own question id, which makes `--field answers.team.choice` and
`--assert 'is_urgent >= 0.7'` work. The full shape is in
[Exit codes and JSON contract](exit-codes.md#the-result-envelope).
