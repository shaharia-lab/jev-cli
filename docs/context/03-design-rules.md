# Designing questions

1. **Code first.** Rules, arithmetic, lookups, dates, counting, side effects stay in code.
2. **One snap judgment per question** — something an expert answers in a second. Anything needing
   reasoning gets decomposed into several questions and recombined in code.
3. **Fan out.** Put every question over the same state in one request, including speculative ones
   whose answer only matters on some branch. State the premise ("If this is a shipping problem, ...").
   Extra questions cost tokens, not latency. Docs' measurement: 13 questions batched ≈ 12× cheaper
   and 10× faster than 13 calls.
4. **Second request only for a real dependency**: you need answer 1 to fetch evidence, build new
   state, or pick the next option set (hierarchy walk, shortlist-then-reread).
5. **Trim state.** Irrelevant context measurably hurts accuracy (context rot). Filter in code first.
6. **Pick the type by what the answer means.** Choice = relative ("which one"); Noul = absolute
   ("does this hold", can be low for every label); Score = position on a described spectrum.
   Multi-label → one Noul per label, not a Choice.
7. **Always offer an exit**: `other` / `none of the above` / `not stated`. The model cannot pick an
   option you omitted.
8. **Score levels describe situations, not degrees.** Each level is judged alone — the model doesn't
   see the number or its neighbours, so "worse than the previous" is meaningless. One dimension per
   Score. Normalize by `len(criteria) - 1` before weighting.
9. **Confusable options → structured criteria** with parallel `what` / `not_for` / `examples`.
   Examples only help when they resemble real inputs.
10. **Gate on uncertainty, scaled to stakes.** `confidence` = how peaked the distribution is, not
    correctness. Noul ≈ 0.5 means "can't tell", never "medium". Several acceptable options also
    flatten a Choice — low confidence isn't always a problem. Ignore confidence on unused branches.
    Tune thresholds on your own data; cookbook numbers are illustrations.
11. **Keep raw judgments, make policy explicit.** Weighted sums for compensating preferences;
    separate conditions for "any serious violation". Reweighting shouldn't need re-inference.
    With labels, feed probabilities into a classical model (CatBoost in the docs).


## Known jagged edges (jev-1.13)

- Literal reader: answers the words, not the intent. If you catch yourself explaining what you
  meant, that explanation belongs in the instruction.
- No counting, arithmetic, numeric closeness (hex colors, RGB), or interpolating a number from a Score.
- Dates are text to it: extract parts as Choices (12 months, 31 days, year range + "not stated"),
  assemble and compare in code.
- Multi-hop / double-negative instructions lose accuracy.
- Prompt injection inside `state` can move answers — be explicit in criteria, test adversarially.
- Contradictory instructions vs criteria (e.g. `true` meaning "no") degrade it.
- No cross-question invariants: P(X) + P(not X) ≠ 1, and a Noul is not comparable to a yes/no
  Choice. Never carry a threshold from one form to the other.
- Generation by chaining choices: don't. Generate candidates by regex/LLM, let Jev select.
