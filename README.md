# exuberance

A trading + AI cockpit for a discretionary options/volatility trader.
**Rust engine, Claude Code brain.**

The strategy: **cheap volatility on proven movers** — find options where implied
vol is underpricing future movement (low IV rank, implied below realized) on
underlyings with a demonstrated history of big moves. See [`CLAUDE.md`](CLAUDE.md)
for the full operating manual.

## Quick start

```bash
cargo test --workspace     # 12 tests, runs offline
cargo run -p exub-cli -- scan   # demo screen over synthetic data
```

Copy `.env.example` → `.env` and add your `POLYGON_API_KEY` when you wire live data.

## The engine (Rust)

| Crate | Role |
|-------|------|
| `vol` | Pure vol math: realized vol, IV rank/percentile, implied−realized spread, move detection. Fully tested, no deps. |
| `market-data` | `Bar`/`IvSnapshot` types + `DataSource` trait. `MockSource` for tests, `PolygonSource` stub for live. |
| `signals` | Screeners. `CheapVolScreen` implements the three-gate strategy. |
| `exub-cli` | The `exub` binary. `exub scan` runs the screen. |

## The brain (Claude Code)

**Skills** — invoke by name:
- `/vol-scan` — run the cheap-vol screen, get ranked candidates.
- `/research-ticker` — deep-dive one name across vol, data, and catalysts.
- `/thesis` — write a trade thesis and stress-test it.

**Subagents** — specialized help: `vol-quant`, `research-analyst`, `risk-manager`,
`devils-advocate`.

**Intended flow:** `/vol-scan` → pick a name → `/research-ticker` → `/thesis`
(auto-invokes risk + devil's advocate) → **you** decide.

## Guardrails

Execution defaults to **paper**. No live orders without an explicit human go.
Secrets live in `.env` (gitignored). This is decision *support* — the human owns
the trade and the risk. Details in [`CLAUDE.md`](CLAUDE.md).

## Roadmap

Vol math + screen + AI layer are done. Next: wire live Polygon data → real
universe scans → journal (watchlist/trade log) → scheduled premarket alerts →
guarded paper execution.
