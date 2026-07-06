---
name: research-ticker
description: Deep-dive a single ticker across vol, data, and catalysts to judge whether cheap implied vol is likely to be realized. Use after /vol-scan surfaces a name, or whenever the user names one ticker to investigate. Arg: the ticker symbol.
---

# /research-ticker — single-name deep dive

Assemble the full picture on one symbol: is this a real cheap-vol opportunity, or
a value trap?

## Steps

1. **Vol picture** — hand the numbers to the `vol-quant` subagent: IV, IV rank,
   realized vol, realized/implied, big-move history. Get the PASS/FAIL verdict on
   the three gates.

2. **Catalyst picture** — hand the ticker to the `research-analyst` subagent:
   what it is, the **catalyst calendar (earnings date is mandatory)**, why it
   historically moves, and an options-liquidity check.

3. **Synthesize.** Put them together: does a plausible catalyst exist to realize
   the vol the options are underpricing, on a chain you can actually trade? Note
   the tension if the vol says "cheap" but there's no catalyst — that's often the
   trap.

Run steps 1 and 2 concurrently (independent subagents) when you can.

## Output

A one-page dossier: the vol verdict, the catalyst calendar, the "why it moves"
read, the liquidity verdict, and a synthesized **conviction: high / medium / low**
with the single biggest reason for and against. End by offering `/thesis <ticker>`
to formalize and stress-test it — or a clear "not worth a thesis, because …".

Evidence and synthesis only; the human decides.
