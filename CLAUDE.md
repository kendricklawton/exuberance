# exuberance

A trading + AI cockpit for a discretionary options/volatility trader. Rust engine,
Claude Code brain. This file is the operating manual — every session and subagent
reads it.

## What we're trading

**The edge: cheap volatility on proven movers.** We hunt options where implied
volatility is underpricing future movement — a positive *variance risk premium*
for the buyer. Concretely, a name qualifies when:

- **IV rank is low** — today's implied vol sits near the bottom of its *own*
  1–3yr range. (An absolute "IV < 30" is meaningless without this; 30 is cheap
  for a biotech, rich for a utility.)
- **Implied < realized** — options imply less movement than the stock has
  actually been delivering (`realized/implied ≥ 1`).
- **Proven mover** — the underlying has made multiple large (≥10%) moves over
  the lookback, so the cheap IV is a genuine mispricing, not a permanently
  sleepy stock.

Desk shorthand for the setup: *"vol is cheap / options are underpricing the move."*
This is a **long-vol / long-gamma** posture. We are option **buyers** here.

Vocabulary we use precisely: *entry criteria / setup* (the rule checklist),
*screen* (the mechanical filter), *investment thesis* (why one specific trade
makes money — distinct from the reusable screen), *mandate* (what we're allowed
to trade at all).

## Repo layout

```
crates/
  vol/          Pure vol math — realized vol, IV rank/percentile, spread, moves. Fully tested, no deps.
  market-data/  Bar/IvSnapshot types + DataSource trait; MockSource (tests) + PolygonSource (stub).
  signals/      Screeners on top of vol + market-data. CheapVolScreen is the flagship.
  exub-cli/     `exub` binary. `exub scan` runs the screen (demo data until Polygon is wired).
.claude/
  agents/       Subagent squad (see below).
  skills/       User-invocable skills: /vol-scan, /research-ticker, /thesis.
```

## Data sources

- **Polygon.io** — the Rust engine's live data path (`PolygonSource`, reads
  `POLYGON_API_KEY`). REST aggregates for bars; options snapshot for IV. Not
  wired yet — it's the next milestone.
- **`massive` MCP tools** — live market data available to *the AI agents* inside
  Claude Code (quotes, chains, IVs, historicals). Use `search_endpoints` then
  `call_api`. Agents lean on this for ad-hoc research today, before the Rust
  Polygon client exists. **Use these MCP tools for any market/price/options
  question — never web search for financial data.**

## The AI layer (this is the point)

**Subagents** (`.claude/agents/`) — hand off specialized work:
- `vol-quant` — computes/screens vol, runs `exub`, reasons about IV rank &
  realized-vs-implied. The numbers guy.
- `research-analyst` — builds the fundamental/catalyst picture for a ticker
  (earnings dates, events, why it moves). Pulls live data via `massive`.
- `risk-manager` — sizes trades, checks against the mandate, flags tail risk,
  earnings landmines, and liquidity. Has veto authority in the workflow.
- `devils-advocate` — adversarially attacks a thesis before capital is committed.
  Its job is to find the reason *not* to trade.

**Skills** (`.claude/skills/`, invoke with `/name`):
- `/vol-scan` — run the cheap-vol screen and return ranked candidates.
- `/research-ticker` — deep-dive one symbol across data, vol, and catalysts.
- `/thesis` — turn a candidate into a written trade thesis with entry/exit/risk,
  then stress-test it with the devil's-advocate.

The intended flow: **/vol-scan → pick a name → /research-ticker → /thesis
(which invokes risk-manager + devils-advocate) → you decide.**

## Guardrails (non-negotiable)

This repo will eventually reach broker execution. Until then and after:

1. **No live orders without an explicit human go.** Execution defaults to paper
   (`EXUB_TRADING_MODE=paper`). Flipping to real money is a deliberate, manual
   act — never inferred, never done by an agent on its own.
2. **Never commit secrets.** Keys live in `.env` (gitignored). `.env.example`
   documents the shape.
3. **Risk-manager is in the loop for anything resembling a trade.** Position
   sizing and mandate checks are not optional.
4. **This is decision *support*, not advice.** Agents surface evidence and
   stress-test theses; the human places the trade and owns the risk.

## Conventions

- **Rust**, edition 2021, workspace crates. Keep `vol`/`market-data`/`signals`
  dependency-light so `cargo test --workspace` runs offline and fast.
- Every new screen/metric ships with unit tests (see `vol` and `signals` for the
  bar). Pure logic is tested against known inputs, not live data.
- `cargo test --workspace` and `cargo clippy --workspace` must be green before
  anything is considered done.
- Money/vol values are **decimals** internally (0.30 == 30%); format to percent
  only at the display edge.

## Build order (roadmap)

1. ✅ Vol math + screen framework + CLI demo (done, tested).
2. ✅ AI layer — CLAUDE.md, subagents, skills (done).
3. ⬜ Wire `PolygonSource` (live bars + IV) behind a `polygon-live` feature.
4. ⬜ Real universe input for `exub scan` + JSON/table output.
5. ⬜ Journal crate: watchlist, trade log, thesis tracking.
6. ⬜ Scheduled premarket scan (Claude Code routine) → alerts.
7. ⬜ Execution crate behind guardrails: paper first (Tradier/Alpaca), gated.
