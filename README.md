# exuberance

A grounded trade-*discovery* engine + AI cockpit for discretionary traders —
**any market, any strategy. It finds and cites; it never recommends and never
acts.** A Rust engine whose **data feeds, strategies, AI models, and brokers all
plug in behind traits**, with an MCP surface.

The engine is **strategy- and asset-class-agnostic** (equities, options, futures,
crypto, FX). Its **flagship reference strategy** — the first that ships and proves
the seam — is *cheap volatility on proven movers*: find options where implied vol is
underpricing future movement (low IV rank, implied below realized) on underlyings
with a demonstrated history of big moves. Other strategies plug in the same way. See
[`.rules`](.rules) for the full operating manual.

## Why this engine exists (vs. Massive's MCP)

Massive (and other vendors) already ship an **MCP server** that hands an AI agent
raw market data — bars, quotes, chains, a live IV snapshot. So why a Rust engine
instead of just letting Claude Code call that MCP? Because **the edge is a
computation, not a data lookup — and the data you need doesn't come from one call.**

(Shown with the flagship vol strategy; every point holds for any strategy.)

- **LLMs can't be trusted to compute the signal.** IV rank (where today's IV sits
  in its *own* 1–3yr range), realized-vs-implied, proven-mover move-counting — exact
  numbers a real-money decision rests on. A chat model eyeballing them from a data
  blob approximates or hallucinates. The engine computes them deterministically and
  **cites the exact inputs** (bars used, history window, provider). The engine
  authors the number; the model never does.
- **A strategy needs accumulated state no snapshot returns.** To rank today's IV
  against its history you need the 1–3yr *series* of daily ATM IV — which a snapshot
  MCP call doesn't give you. The engine **accumulates and persists** that state; every
  strategy has state like it, and without the engine you can't compute the signal at
  all. *This is the concrete reason to exist.*
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
computes the edge (cheap-vol today, any strategy next) — correctly, statefully,
reproducibly, vendor-agnostically — and exposes *that* as a grounded tool.

## Quick start

```bash
cargo test --workspace          # 30 tests, runs offline
cargo run -p cli -- scan        # demo screen over synthetic data (evidence, not advice)
cargo run -p cli -- providers   # the plug-in catalog: data feeds, AI models, coding agents, brokers
cargo xtask ci                  # the full local gate (fmt, clippy, build, test, docs, powerset, deny)
```

`exub providers` shows the multi-vendor **plug-in matrix** — every data feed
(mock, Massive, Alpha Vantage), AI model (Claude, Gemini, OpenAI), coding
agent (Claude Code, Gemini CLI, Codex), and broker — with `wired`/`planned`
status. Selecting one is config (`--data-provider`, `EXUB_DATA_PROVIDER`); adding
one is a new adapter + one registry arm. The seams are `async` so real feeds and
models slot straight in.

Config is layered **flags > env (`EXUB_*`) > file (TOML) > defaults** — e.g.
`EXUB_DATA_PROVIDER=massive`, `EXUB_TRADING_MODE=paper`, `--config exub.toml`.
Secrets never live in config: copy `.env.example` → `.env` and add your
`MASSIVE_API_KEY` (env only) when you wire live data.

## The engine (Rust)

The engine is **agnostic across four seams**: it talks to market-data feeds,
brokers, AI models, and strategies only through traits in `exub-core`, so swapping a
feed, broker, model, or strategy is adding a crate, not editing the engine. See
[`ROADMAP.md`](ROADMAP.md).

| Crate | Role |
|-------|------|
| `exub-core` | The contract layer: `Provider`/`Capability`, the unified `ProviderError`, and the `MarketDataProvider` / `BrokerProvider` / `AiProvider` seams + mock/paper/echo reference impls. Only dep: `async-trait`. |
| `vol` | Vol math for the flagship strategy: realized vol, IV rank/percentile, implied−realized spread, move detection. Fully tested, no deps. |
| `market-data` | Market-data **providers** implementing `exub-core`'s trait. `MockSource` for tests, `MassiveSource` stub for live. |
| `signals` | Pluggable **strategies/screens** over any `MarketDataProvider`. Cheap-vol is the flagship reference strategy; others plug in behind the same seam. |
| `cli` | The `exub` binary. `exub scan` runs the screen; `exub providers` lists the wired providers. |
| `xtask` | Dev orchestration — `cargo xtask ci` runs the full local gate. Never shipped. |

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
  harness-agnostic. See [`ROADMAP.md`](ROADMAP.md) Phases 15–18.

## Guardrails

Execution defaults to **paper**. No live orders without an explicit human go.
Secrets live in `.env` (gitignored). This is decision *support* — the human owns
the trade and the risk. Details in [`.rules`](.rules).

## Roadmap

The arc: vol math + screen → provider-agnostic contracts → config/CI → async seams +
registry → live data feeds → the IV-history store (the "reason to exist") → the
in-engine AI layer + MCP surface → guarded paper execution. The full 26-phase plan is
in [`ROADMAP.md`](ROADMAP.md) — its checkboxes are the **single source of truth** for
progress.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) — prerequisites, the local gate
(`cargo xtask ci`), the testing approach, and the invariants (agnostic-by-trait,
finds-not-recommends, no-panic, secrets-out-of-repo). The operating manual is
[`.rules`](.rules).

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
