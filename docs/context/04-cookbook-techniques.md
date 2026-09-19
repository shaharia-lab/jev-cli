# Cookbook techniques

Cookbooks run `jev-1.12` with `typesafe-sdk>=0.5.7` plus a helper package `cooksafe` (`JsonCache`,
`make_playground_link`) from `https://pypi.typesafe.ai/`. Numbers below are the cookbooks' own, on
their data — illustrations, not guarantees.

**Tagged-line ids instead of index paths.** Backticked paths appear in the concept pages and the SDE
cascade, always pointing at *named keys*, never at array positions. For anything list-shaped the
cookbooks prefix each line with an id inside a plain-string state and name the id
in the question. This is the fix for the long-array problem found in live testing (see `05-live-test-findings.md`).

```python
DOCUMENT = "\n".join(f"L{i:03d}| {line}" for i, line in enumerate(LINES))
Choice(instructions=f'Which line of the document contains the answer to: "{query}"?',
       criteria={f"L{i:03d}": None for i in range(len(LINES))})   # descriptions None: text is in state
```

**Parallel questions** (13 questions over a 54k-char article): batched `$0.000497 / 0.27 s` vs 13
calls `$0.006090 / 2.71 s`. Answers identical either way. Two of eight Nouls showed run-to-run noise
(std ≈ 0.005) regardless of batching — never park a threshold right on a value. Saving approaches N×
as the state grows. Roughly 4.5 chars per token.

**Re-ranking** (BM25 top-30 → one Noul per query–candidate pair, 1,200 calls, 12 threads): top-1
5%→18%, top-10 38%→62%, total `$0.0645`. One request per pair so scores are absolute and comparable;
sort by `noul`, no threshold. The `false` criterion names the confusable near-miss ("merely on a
similar topic"). Reranking can't recover what stage 1 missed.

**Line-by-line search**: one request, a 218-option Choice to rank lines plus a Noul "does any line
answer this?" — needed because Choice probabilities always sum to 1, so some line always wins (a
query with no answer still gave a top line 0.86 while the existence Noul said 0.14). Bands used:
≥0.7 found, <0.35 absent, between = partial. Beyond 255 lines: one Choice picks a window, a second
ranks inside it.

**Structure recovery** (plain text → Markdown, zero generation): pass 1 = one Noul per adjacent line
pair ("does this line continue the previous sentence?"); pass 2 = per block a type Choice plus
speculative companions (heading level, ordered-step Noul, callout kind). 62 questions in 0.51 s.
- Code handles direct evidence first (blank lines, `- `, `1.`, `#`) and never asks about it.
- **Let code split the population before thresholding**: join bar is 0.2 after a dangling line end,
  0.5 after terminal punctuation — no single threshold separated both cases.
- **Name the narrowest deciding fact.** "Same paragraph?" scored list items 0.77–0.91; "picks up
  mid-sentence?" scored them 0.05–0.22.
- Group decisions come from aggregates: a list is numbered when the *mean* step Noul ≥ 0.5.

**Function calling** (10 functions, 54 questions, one request per command): a routing Choice plus
argument questions for *every* function; the dispatcher reads only the winner's. Derived from type
hints: `Literal[...]` → Choice whose option keys are the exact argument strings; `list[Literal]` →
one Noul per member; `bool` → Noul. Ints, dates, free text get no question and keep defaults.
- Each argument also gets a **"was this stated?" Noul**; if no, omit the argument so the default
  applies — otherwise the Choice confidently invents a value.
- Call confidence = **min** over the judgments used, not the product.
- Write questions about the idea, not the parameter name; when two args share a domain, spell out
  the roles ("the one being measured, named first" vs "the yardstick").

`confidence` is not the max probability — cookbooks show 0.27 on a 3-option Choice. Use
`probabilities` directly when you need a specific statistic.

**Self-consistency** (Nouls and Choices): answers are *not* bit-deterministic. A 14-Noul rubric
(one call, 111 ms, `$0.000043`) showed std ≈ 0.01, one Noul ranging 0.43–0.53 across runs. Fix is an
explicit abstain band, not retries: Noul in 0.30–0.70 → `uncertain` → human. For an 8-Choice
moderation rubric, raw label agreement was 90.8%; adding "top probability < 0.60 → `uncertain`" gave
99.2% agreement with 74.2% handled automatically and 25.8% abstained.

**LLM guardrails**: one request per message, on both input and output — 4 hazard Nouls with
`true`/`false` criteria plus one 4-level severity Score. Routing is a named policy in code: review
≥ 0.35, act ≥ 0.70 (strict) or ≥ 0.85 (permissive); severity ≥ 2.0 upgrades review to block;
precedence support > block > review > pass.

**SDE cascade** (cheap LLM extracts → Jev verifies → big LLM only on flags): per-field Noul heads
framed so that **"bad" = true** (`hallucinated`, `wrong_field`, ...), aggregated with **max** and
gated at 0.7 to escalate. `instructions` is a dict carrying `extracted_field` and `field_spec`, and
the question text references those keys with backticks — so backticked paths resolve against the
instruction object as well as the state. A single holistic "is this extraction good?" judge scored
0.56 on a field where the specific `hallucinated` head scored 0.95: decomposed heads catch what a
holistic judge blurs.

**Date extraction**: 7 Choices in one call — `mode, month, day, year, day_anchor, weekday,
week_offset`. Year is a ~153-option Choice with `none` and `out_of_range` escapes. Code does all
calendar math; confidence = min over the parts actually used; < 0.60 → review.

**Pre-parsed value extraction**: regex deliberately over-finds candidate spans, a Choice picks among
them plus `none`, code copies the winner verbatim and normalises. Above 255 candidates, narrow in
two stages.

**Hierarchical classification**: each sibling set is one Choice with options keyed `c0..cN` (short
neutral keys, descriptions carry the meaning). Beam search K=3, path score
`product(edge_probabilities) ** (1 / decisions)` (geometric mean, so deep paths aren't penalised):
4/4 leaves vs 2/4 for greedy. Note the shipped code sends one request per beam path in a thread
pool even though the prose describes K questions per call.

**Autoresearch feature discovery**: an LLM proposes 5-level Score and Noul questions, Jev answers
them over 2,000 rows, each answer becomes mean + sd feature columns for CatBoost. RMSE 3.09 (mean
baseline) → 2.15 (asking Jev for the target directly) → 1.87 (round 1) → 1.77 (round 5, 38
questions). Hard facts surfaced here: **an 11-level Score returns a server error**, and **8
concurrent workers were "already enough to hit a rate limit on a shared key"**.

**Classification using confidence**: one 75-option Choice per SEC filing; "a Choice works reliably
up to roughly 240 options". Confidence ≥ 0.9 → report the fine group (27/30 right); below → back
off to the parent division in code, lifting the unsure half from 40% to 70% and overall 39/60 →
48/60. Backing off to a coarser label is a cheap alternative to abstaining.

**Recurring rules across cookbooks**: every question about a state in one request; routing lives in
code as named threshold policies; generate Choice options in code and always include a `none`
escape; aggregate with min (all must hold) or max (any violation), rarely mean; log
`response.model`; design around 255 options / 10 levels / rate limits.

**Skill suggestion** (pick ≤1 of 182 agent skills per turn; directly relevant to harness tooling):
request 1 = one Choice over all 182 names (criteria = 60-char descriptions) + three gate Nouls on
whether a skill is needed at all; request 2 = Choice over the top 3 with rich criteria (full
description + first 700 chars of SKILL.md) + one absolute `fits::<name>` Noul per candidate.
- Gate = mean of three differently-phrased Nouls, with the inverted one (`prose_suffices`) flipped;
  < 0.30 → suggest nothing. `max(fits) < 0.30` → drop the whole shortlist.
- The gate must ask **action vs explanation, not topic** — "explain what a monad is" and a real
  coding task are both "software".
- Inject the winner as one ignorable line *after* the cacheable roster prefix; on no-match still
  send an explicit "no skill appears relevant" sentence.
- Wrong loads 16.8% → 7.3%, needless loads 9.8% → 4.0% (oracle floor 2.5% / 1.2%). Latency 0.16–0.31 s
  wide, ~0.1 s rerank. The second pass can only reject what the first surfaced.

**Entity alignment**: one request per candidate pair, state `{entity_a, entity_b}`. A 3-level Score
whose **levels are the three actions** (different / maybe-variant / same → unlinked / curator queue /
assert sameAs), routed by nearest level `int(score + 0.5)` — no fitted threshold; you tune by
rewording the middle level. Three per-field Nouls (name, brewery, style) explain disagreements to the
curator. Numeric fields (abv) get no question. Needs a cheap blocking pass first; no accuracy reported.

**Classifying RAG passages**: one request per (query, passage), four narrow Nouls — relevant,
contains answer evidence, contradicts the query's premise, attempts to control the answering system.
None asks "include it?"; that is an ordered rule in code, first match wins:
injection > 0.70 → exclude; contradiction > 0.70 → `conflicting_evidence`; relevance < 0.45 → exclude;
evidence > 0.55 → include; else exclude. Injection first (security), contradiction before evidence
(a premise-denying passage would otherwise be dropped as weak evidence). Re-routing on cached answers
costs nothing. The planted injection ranked **1st** by embedding similarity and scored 0.99. The page
is explicit that this is **not a security boundary**; the generator still treats passages as untrusted.

**Citation check**: code first — normalized exact substring match (fold curly quotes, collapse
whitespace, strip page furniture); a miss is `fabricated` with no model call, and the match also
locates the section so the state stays small. Then one 3-option Choice on `{claim, section}`:
`supports` / `contradicts` / `says_nothing`; `confidence ≥ 0.8` auto-accepts, else human. A verbatim
quote is not support — one quoted sentence sat in a section that contradicted the claim (0.99).

**Option keys = program values.** Literal strings, skill names, line ids, verdict keys — no mapping
layer afterwards, and output is always inside the closed set.

**Ops.** Cookbooks run 4–12 workers and note "the public endpoint rate-limits above roughly eight".
Cache by arguments *including the question JSON* so rewording invalidates; cache tokens and latency,
derive dollars later. Deterministic ordering and tie-breaks make replays stable.

Gaps in the docs: no stated max questions per request beyond the token budget; function-calling
internals (`stated` threshold, per-argument probability formula) aren't shown; the structure-recovery
page contradicts itself on cost (`$0.0003` printed vs `$0.0015` in prose — the former matches the
token math).
