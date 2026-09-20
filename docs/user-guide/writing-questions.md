# Writing good questions

Jev makes quick, literal judgments about meaning. Most bad answers come from questions that ask
for something else. These habits cost nothing and change the results.

For deeper question design, read TypeSafe's own documentation at <https://docs.typesafe.ai> and
their agent skill, [typesafe-ai/skills](https://github.com/typesafe-ai/skills).

## Keep computation in your code

Counting, arithmetic, comparing numbers or dates, lookups and exact matching belong in your
program. Ask Jev for the meaning, then compute in code.

| Instead of asking | Ask |
| --- | --- |
| "Is this invoice more than 30 days old?" | A choice for the month and one for the day, then compare in code |
| "How many products are mentioned?" | "Is more than one product mentioned?" as a noul, or extract in code |
| "Is this the highest priority ticket?" | A score per ticket, then rank in code |

## One quick judgment per question

If a knowledgeable person could not answer it at a glance, split it. Two clear questions
combined in code beat one question that needs reasoning.

## Ask everything in one request

Extra questions add tokens, not round trips, and the state is paid for once. Include questions
you only need on some branches and phrase them with their premise, for example "If this is a
billing problem, is a refund being requested?". Make a second call only when the question
genuinely depends on the first answer.

## Send only what matters

Trim the state to the part the questions are about. Unrelated text lowers accuracy and costs
money. A state can be plain text or structured JSON, whichever is closer to the truth.

## Say exactly what you mean

Jev answers the words, not the intent. Avoid double negatives, and never let criteria contradict
the instructions. Write criteria as observable facts about the content:

```yaml
questions:
  mentions_outage:
    type: noul
    instructions: Does the message report that a service is unavailable right now?
    criteria:
      "true": The writer states that something is down, failing, or unreachable at this moment
      "false": No current failure is reported, or the problem is described as already fixed
```

## Give every choice a way out

Add `other`, `none_of_the_above` or `not_stated`. Without one, the model must pick from options
that may all be wrong.

## Describe score levels as situations

Each level must be recognisable on its own, because levels are judged separately. "Worse than
level 1" means nothing to the model. Keep one dimension per score and between 2 and 10 levels.

## Treat the state as untrusted

The content you send may itself contain instructions, especially if it came from a user. Jev
answers the question it was given, but explicit criteria make it much harder to steer. Test with
hostile inputs when the stakes are high, and never build an action that only a probability
authorises.

## Tune thresholds on your own examples

Collect a few dozen cases you already know the answer to, run them through
[`jev batch run`](batch.md), and pick the threshold that gets the trade-off you want. A
threshold copied from a blog post is a guess. Keep the raw probabilities, so you can change the
policy without asking the model again.

## Check before you spend

```bash
jev validate -f triage.yaml --state-file ticket.txt --strict
jev eval -f triage.yaml --state-file ticket.txt --dry-run
```

`jev validate` reports everything wrong at once, each with a rule id, the path in the file and a
suggested fix, and it warns about the mistakes above: a choice with no escape option, a score
with one level, criteria that repeat the instructions. `--strict` turns those warnings into
failures, which is what a CI job should use. Neither command sends anything or needs a key.

## When the model cannot tell

A probability near 0.5 is not "medium", it is "cannot tell". Route those cases to a person:

```bash
jev noul "Is this message unsolicited advertising?" --state-file message.txt \
  --fail-under 0.6 --abstain-band 0.4,0.6
```

Exit 11 means "inside the band". [Scripting and CI](scripting-and-ci.md) shows the whole pattern.
