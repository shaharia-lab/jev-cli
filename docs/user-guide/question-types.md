# The four question types

Jev answers four kinds of question. Picking the right one is most of the work, because the
answer's shape decides what your code can do with it.

| Type | Ask it when | You get back |
| --- | --- | --- |
| `noul` | A property either holds or it does not | The probability of yes, from 0 to 1 |
| `choice` | Exactly one of up to 255 options must win | The winner, its confidence, and a probability per option |
| `score` | A described scale of 2 to 10 levels | A position on the scale, and a confidence |
| `box` | Something has to be located in an image | The box's four edges, and a confidence |

The first three have a shortcut command for one question (`jev noul`, `jev choice`,
`jev score`); any of them can be one of many questions in a request file run with `jev eval`,
which is also where a `box` question goes.

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

## box: where is this in the image?

```yaml
# find-mug.yaml
questions:
  mug:
    type: box
    instructions: the coffee mug in `photo`
  present:
    type: noul
    instructions: Is there a coffee mug in `photo`?
```

```bash
jev eval -f find-mug.yaml --image photo=desk.jpg --model jev-preview -o json
```

`--image NAME=PATH` attaches a PNG or JPEG and puts it under `NAME` in the state, so questions
refer to it as `` `NAME` `` like any other part of the state; repeat the flag for several images.
The answer is `box` with `top`, `bottom`, `left` and `right` as fractions of the image's height
and width (0 is the top left corner, 1 the bottom right), plus a `confidence`. Three things to
know:

- **It needs Jev 1.14.** Pass `--model jev-preview` (or a versioned id from 1.14 up); older
  models reject the request. Images and boxes are experimental on the API side, so the shape can
  change between releases.
- **There is no "not found".** A box question always answers with four edges, even when the
  thing is not in the picture. Pair it with a noul, as above, and ignore the box when the noul
  is low.
- **One target per question.** To find several things, ask one box question for each. Small
  things in a large picture can be missed; crop to the region first.

The API accepts PNG and JPEG only, each side 256 to 1568 pixels and 78,400 to 2,458,624
pixels in all, and rejects the whole request over one bad image rather than resizing it. `jev`
reads the size from the file's header and refuses it offline, before anything is sent.

Gate on it with `--assert "mug.confidence >= 0.8"`, and read the edges with
`--field answers.mug.box`. The size estimate counts the text only; image tokens are billed on
top, and `usage.input_tokens` in the result shows the real figure.

A request file can also carry the image itself, base64 encoded, under `assets`, keyed by the
reference string written where the image belongs:

```yaml
state:
  photo: img-7c2f9a
questions:
  mug: { type: box, instructions: "the coffee mug in `photo`" }
assets:
  img-7c2f9a: { type: image, data_base64: iVBORw0KGgoAAAANSUhEUgAAAZAAAAEsCAIAAABi1XKV, format: png }
```

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
