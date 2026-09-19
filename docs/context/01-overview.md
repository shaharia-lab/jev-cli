# Overview

> Study notes on TypeSafe AI's Jev model, written 2026-09-19 from https://docs.typesafe.ai.
> The upstream docs are the source of truth and change quickly; run `scripts/fetch-upstream-docs.sh` for a fresh local mirror.

## What it is

Jev is a "System One" model: it reads natural language but **never generates text**. You send a
`state` plus a map of typed questions; it returns a probability distribution over answers *you*
defined. Code owns control flow; Jev supplies the semantic judgment inside an `if`.

- Trained with RLCD (RL for calibrated decisions): a 0.8 should be right ~80% of the time across a
  group of predictions. Not a per-answer guarantee.
- Same weights for every account. No fine-tuning — you adapt via `state`, `instructions`, `criteria`.
- Not trained on customer data. ZDR is enterprise-only.
- English is the strong language; others (incl. CJK) work but worse.
- Text only. No images/audio/video.
