---
name: vol-scan
description: Run the cheap-vol / proven-mover screen and return ranked candidates. Use when the user wants to find underpriced-volatility options — names where IV is cheap for the name, below realized vol, on a proven mover. Optional args: a universe (index/sector/ticker list) and criteria overrides.
---

# /vol-scan — cheap-vol screener

Find options where implied vol is underpricing movement on a proven mover.
See CLAUDE.md for the full strategy.

## Steps

1. **Determine the universe.** Use tickers from the user's args if given;
   otherwise ask for a universe (index, sector, or watchlist) — don't guess a
   giant list silently. Note any cap you apply.

2. **Pull the data** via the `massive` MCP tools (`search_endpoints` → `call_api`;
   never web search for prices/IV). For each symbol you need:
   - current ATM/30-day implied vol and its trailing history (for IV rank),
   - ~3 years of daily closes (for realized vol and big-move counting).

3. **Apply the three gates** (delegate the math to the `vol-quant` subagent, or
   run `exub scan` once Polygon is wired):
   - IV rank ≤ 0.20 (cheap for the name),
   - realized/implied ≥ 1.0 (moved more than options imply),
   - ≥ 2 daily moves of ≥ 10% over the lookback (proven mover).
   Criteria are overridable from args (e.g. "IV rank under 0.15").

4. **Rank** passers by most-underpriced first (most negative implied−realized
   spread).

## Output

A table: SYMBOL · IV · IV rank · realized vol · realized/implied · big moves ·
max move. Then a one-line read on the top 1–3 names. State the universe size and
any cap applied — never imply full coverage you didn't do. Close by offering
`/research-ticker <top name>` as the next step.

This surfaces candidates only. It is not trade advice.
