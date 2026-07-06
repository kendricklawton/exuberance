---
name: thesis
description: Turn a candidate into a written, stress-tested options trade thesis — structure, entry/exit, sizing, risk — then attack it before the human decides. Use when the user is ready to formalize a trade on a researched name. Arg: the ticker (and optional structure idea).
---

# /thesis — write and stress-test a trade thesis

Formalize a cheap-vol trade into a concrete, falsifiable plan, then try hard to
break it. Nothing here places an order.

## Steps

1. **Draft the thesis** (pull any missing vol/catalyst facts via `/research-ticker`
   or the subagents first):
   - **Claim** — one sentence: why this makes money (the specific move the market
     is underpricing, and the catalyst to realize it).
   - **Structure** — long calls/puts/straddle/etc., strike, expiry. Expiry must
     give the catalyst room to play out. Note if an earnings print sits inside it.
   - **Entry** — price/IV level to enter at.
   - **Exit** — profit target and the invalidation (what proves the thesis wrong
     and gets you out). A thesis without an exit is not a thesis.
   - **Time horizon.**

2. **Risk gate** — hand the draft to the `risk-manager` subagent for sizing, the
   earnings/IV-crush check, liquidity, mandate, and an APPROVE / CONDITIONS / VETO.

3. **Adversarial gate** — hand it to the `devils-advocate` subagent to build the
   strongest case *against*. Surface its single best objection prominently.

Run steps 2 and 3 concurrently.

## Output

The written thesis (claim · structure · entry · exit · horizon · max loss), then
the **risk-manager verdict** and the **devil's-advocate's strongest objection**,
then a synthesized bottom line: does the thesis survive both gates? If yes, it's
ready for the human's decision. If a gate vetoed, say so and what would change it.

This is decision support. The human places the trade and owns the risk (CLAUDE.md
guardrails). Never route or imply routing a live order.
