# exuberance

A trading + AI cockpit for a discretionary options/volatility trader.
**A Rust engine with a pluggable AI layer** — any model or coding agent behind one
trait, and an MCP surface — not a set of tool-specific personas.

The strategy: **cheap volatility on proven movers** — find options where implied
vol is underpricing future movement (low IV rank, implied below realized) on
underlyings with a demonstrated history of big moves. See [`CLAUDE.md`](CLAUDE.md)
for the full operating manual.

## Why this engine exists (vs. Massive's MCP)

Massive (and other vendors) already ship an **MCP server** that hands an AI agent
raw market data — bars, quotes, chains, a live IV snapshot. So why a Rust engine
instead of just letting Claude Code call that MCP? Because **the edge is a
computation, not a data lookup — and the data you need doesn't come from one call.**

- **LLMs can't be trusted to compute the signal.** IV rank (where today's IV sits
  in its *own* 1–3yr range), realized-vs-implied, proven-mover move-counting — exact
  numbers a real-money decision rests on. A chat model eyeballing them from a data
  blob approximates or hallucinates. The engine computes them deterministically and
  **cites the exact inputs** (bars used, IV-history window, provider). The engine
  authors the number; the model never does.
- **IV rank needs accumulated state no snapshot returns.** To rank today's IV
  against its history you need the 1–3yr *series* of daily ATM IV — which a snapshot
  MCP call doesn't give you. The engine **accumulates and persists** that history;
  without it you can't compute the core signal at all. *This is the concrete reason
  to exist.*
- **Scale, schedule, reproducibility, vendor-independence.** Premarket-scan a
  universe, rank, diff a watchlist, alert; backtest the screen to know the edge is
  real; do it identically every day across whichever feed is available — Massive
  today, Alpha Vantage tomorrow — behind one schema, drift caught in CI. A chat
  session is none of these.
- **Then it becomes the MCP.** exuberance exposes a *higher-order* tool — "cheap-vol
  candidates, with cited evidence" — that Claude Code / Gemini CLI / Codex call.
  Their LLM reasons about *which* trade; the engine supplies the trustworthy,
  computed signal. Massive's MCP cites nothing; ours cites the number and its
  provenance.

**In one line:** Massive's MCP gives an agent raw data to (mis)crunch; exuberance
computes the vol edge — correctly, statefully, reproducibly, vendor-agnostically —
and exposes *that* as a grounded tool.

## Quick start

```bash
cargo test --workspace          # 30 tests, runs offline
cargo run -p cli -- scan        # demo screen over synthetic data (evidence, not advice)
cargo run -p cli -- providers   # the plug-in catalog: data feeds, AI models, coding agents, brokers
cargo xtask ci                  # the full local gate (fmt, clippy, build, test, docs, deny)
```

`exub providers` shows the multi-vendor **plug-in matrix** — every data feed
(mock, Massive/Polygon, Alpha Vantage), AI model (Claude, Gemini, OpenAI), coding
agent (Claude Code, Gemini CLI, Codex), and broker — with `wired`/`planned`
status. Selecting one is config (`--data-provider`, `EXUB_DATA_PROVIDER`); adding
one is a new adapter + one registry arm. The seams are `async` so real feeds and
models slot straight in.

Config is layered **flags > env (`EXUB_*`) > file (TOML) > defaults** — e.g.
`EXUB_DATA_PROVIDER=polygon`, `EXUB_TRADING_MODE=paper`, `--config exub.toml`.
Secrets never live in config: copy `.env.example` → `.env` and add your
`POLYGON_API_KEY` (env only) when you wire live data.

## The engine (Rust)

The engine is **provider-agnostic**: trading logic talks to market-data, broker,
and AI vendors only through traits in `exub-core`, so swapping a feed, broker, or
model is adding a crate, not editing the engine. See [`ROADMAP.md`](ROADMAP.md).

| Crate | Role |
|-------|------|
| `exub-core` | Provider-agnostic contracts: `Provider`/`Capability`, `ProviderError`, and the `MarketDataProvider`, `BrokerProvider`, `AiProvider` traits + mock/paper/echo reference impls. Dependency-free. |
| `vol` | Pure vol math: realized vol, IV rank/percentile, implied−realized spread, move detection. Fully tested, no deps. |
| `market-data` | Market-data **providers** implementing `core`'s trait. `MockSource` for tests, `PolygonSource` stub for live. |
| `signals` | Screeners over any `MarketDataProvider`. `CheapVolScreen` implements the three-gate strategy. |
| `cli` | The `exub` binary. `exub scan` runs the screen; `exub providers` lists the wired providers. |

## The AI layer (in the engine)

The AI layer lives **inside the engine**, not as tool-specific personas:

- **Model + agent adapters behind one seam.** Any LLM (Claude, Gemini, OpenAI) and
  any coding agent (Claude Code, Gemini CLI, Codex) plug in behind `AiProvider`,
  selected by config. The model *plans* a query; the **engine** fetches, computes,
  and cites the number — so a figure can't be hallucinated.
- **Exposed via MCP.** The engine will publish its grounded discovery capabilities
  (scan, evaluate, backtest) as an MCP tool-server, so any agentic assistant calls
  *cited signals* instead of crunching raw data itself.
- **The desk process as a cited pipeline.** scan → research → thesis → risk-check →
  adversarial review becomes a reproducible, logged, cited engine pipeline —
  harness-agnostic. See [`ROADMAP.md`](ROADMAP.md) Track D.

## Guardrails

Execution defaults to **paper**. No live orders without an explicit human go.
Secrets live in `.env` (gitignored). This is decision *support* — the human owns
the trade and the risk. Details in [`CLAUDE.md`](CLAUDE.md).

## Roadmap

Vol math + screen, provider-agnostic contracts, config/CI, async seams + registry,
and schema/error hardening are in. Next: live data feeds → the IV-history store
(the "reason to exist") → the in-engine AI layer + MCP surface → guarded paper
execution. Full 26-phase plan with live status in [`ROADMAP.md`](ROADMAP.md).
