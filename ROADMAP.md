# Roadmap — exuberance: a grounded trade-*discovery* engine (Rust)

**exuberance helps a discretionary options/vol trader *find* trades. It finds and
cites; it never recommends and never acts.** You ask where volatility is cheap on
a proven mover, and the engine surfaces candidates plus the evidence — grounded in
real data, with its provenance — so *you* decide. This is decision **support**, not
advice, and not autonomous execution.

The engine is **provider-agnostic**: it plugs any market-data feed, any AI model,
and (for human-initiated execution only) any broker in behind three small traits,
so trading logic never names a vendor. This is the staged plan; the *why* is in
[`ARCHITECTURE.md`](./ARCHITECTURE.md) (to be written), the operating manual in
[`CLAUDE.md`](./CLAUDE.md).

> **Legend:** ✅ done · 🚧 in progress · ⬜ not started. This plan is written from a
> **clean slate — every phase below is ⬜ (not started).** **26 phases** across six
> tracks; phases within a track are ordered, tracks overlap. Every phase closes
> with tests + `cargo clippy` clean + a working demo — that's the definition of
> done, not a nice-to-have.

## §0 The spine — one headless engine, three ports, pure-view surfaces

This is **ports & adapters** (hexagonal). A headless **engine** answers a discovery
question by driving three *ports* — a [`MarketDataProvider`], an [`AiProvider`],
and (later, guarded) a [`BrokerProvider`] — and every **surface** (the `exub` CLI
now; an API later) only *renders* what the engine found. No surface calls a feed,
a model, or a broker directly.

```
   adapters (ports)                 core (the headless engine)             surfaces
 ┌────────────────────────┐ screen ┌───────────────────────────────┐ read ┌──────────────┐
 │ Data:   polygon/…/mock  │◀────▶ │  screen → ground → cite;        │ ───▶ │ CLI  `exub`   │
 │ AI:     claude/…/mock   │◀────▶ │  canonical schema. The engine   │ ───▶ │ API  (later)  │
 │ Broker: paper/…  (gated)│◀────▶ │  authors the numbers, not the   │      └──────────────┘
 └────────────────────────┘        │  model; never a recommendation. │   every surface renders
   raw APIs mapped to the          └───────────────────────────────┘   grounded candidates + evidence
   canonical schema here
```

The flow is one-directional: **question → the engine screens/computes over
canonical data → it surfaces candidates and the evidence that justifies them,
each cited → the surface renders it.** The engine — not the LLM — authors every
figure (IV rank, realized/implied, move count), so a number can't drift into a
hallucination, and no layer ever emits "buy this."

## §0.5 The engineering contract (what every phase leans on)

1. **Ports & adapters.** A new feed, model, or broker is a **new adapter behind a
   trait and nothing else**; the core depends on none of them. Adapters are
   feature-gated so a lean build compiles without every vendor SDK.
2. **Grounded discovery, not advice.** The engine answers from data it actually
   fetched, computes the figure itself, and reports provenance (metric, value,
   bars/IV history used, provider). It surfaces *candidates + evidence*, never a
   recommendation or a verdict. A **grounding check** ships with the first real
   adapter, not at the end.
3. **Canonical schema = anti-corruption layer.** Providers map raw API → the
   engine's canonical types (`Bar`, `IvSnapshot`, screen results); the engine
   never sees raw. This contains provider **API drift** to one adapter.
4. **Drift caught in CI.** Every real adapter has **contract tests over recorded
   fixtures** (deterministic, offline). A provider/model contract change fails
   CI, not a premarket scan.
5. **Capabilities, declared.** Each provider states what it can serve (bars, IV,
   options chain, …); the engine only plans a query a provider can answer, and
   fails fast and clearly otherwise.
6. **Async + streaming seams.** The seams are `async` (`tokio`, object-safe via
   `async-trait` so the engine can hold `Box<dyn …>` chosen at runtime); results
   stream to the surface as they're found.
7. **Mock-first, keyless.** A permanent mock feed + mock model make everything
   build, test, and demo with **no API keys, no network** — and are the basis for
   deterministic **known-answer evals** of the screens.
8. **Twelve-factor & secrets-out-of-repo.** Layered config **flags > env
   (`EXUB_*`) > file (TOML) > defaults**; adapter selection is config, not code.
   Secrets come from **provider-native env vars only** (`POLYGON_API_KEY`,
   `ANTHROPIC_API_KEY`, …) — never the config file, a log, or a fixture; held in
   `secrecy::SecretString` at the adapter edge.
9. **Structured errors, no panics.** One `ProviderError` vocabulary
   (NotFound/Auth/RateLimited/Transport/Unsupported/…); `unwrap`/`expect`/`panic!`
   denied outside tests — a failed feed/model/broker call is a value that degrades
   to a clear message.

## §0.55 Why this engine exists (the differentiator — read before prioritizing)

Massive (and others) already expose raw market data to agents over **MCP**, so raw
*access* is not a reason to build. The engine earns its existence by what it does
*on top* of any feed — and the build order must center that, not raw-feed wiring:

1. **Deterministic, cited computation an LLM can't be trusted to do.** IV rank,
   realized/implied, move-counting — the engine authors these numbers and cites
   their inputs; the model never eyeballs them. *(The whole of Track C.)*
2. **Accumulated IV-history state no snapshot returns** (Phase 8 ⭐). IV rank needs
   the 1–3yr IV *series*; a snapshot MCP call can't give it. The engine persists it,
   so the core signal is computable *only because the engine exists*. **This is the
   reason to exist — prioritize it over a bare feed.**
3. **The engine becomes a higher-order MCP** (Phase 17 ⭐): it exposes *grounded,
   cited signals* ("cheap-vol candidates + evidence") to Claude Code / Gemini CLI /
   Codex — the mirror of Massive's raw-data MCP. Their LLM picks the trade; our
   engine supplies the trustworthy number.

Corollary for sequencing: a live feed (Phase 7) is **plumbing** whose value is only
realized by Phases 8 + 17. If a change turns the engine into a thin passthrough to a
vendor's data, it has lost its reason to exist.

## §0.6 Guardrails (non-negotiable — from CLAUDE.md)

The discovery engine never crosses into acting. These are enforced by types +
tests, not convention:

1. **No live orders without an explicit human go.** Execution defaults to paper
   (`EXUB_TRADING_MODE=paper`); flipping to real money is a deliberate, manual,
   multi-step act — never inferred, never done by the engine or an agent.
2. **A risk check in the loop for anything resembling a trade** (sizing, mandate,
   tail/earnings/liquidity checks) — the mechanical in-code risk engine (Phase 21),
   with veto.
3. **Decision support, not advice.** The engine surfaces evidence and stress-tests
   theses; the human places the trade and owns the risk.
4. **Never commit secrets.** Keys in `.env` (gitignored); `.env.example` documents
   the shape.

## Phase index

1 vol math + screen + CLI · 2 ~~AI layer~~ (dropped → Track D) · 3 provider
contracts · 4 config/CI/xtask ·
5 async seams + registry · 6 canonical schema & errors · 7 Polygon data ·
8 IV history + storage ⭐ · 9 second feed + cache · 10 universe & symbology ·
11 screen output · 12 more screens (skew/term) · 13 backtest · 14 screen evals ·
15 AI seam depth · 16 real AI adapters · 17 engine↔agent + MCP ⭐ · 18 thesis
pipeline · 19 broker seam · 20 paper fills · 21 risk engine · 22 live gate ·
23 journal · 24 scheduled scan · 25 packaging/signing · 26 CLI/TUI cockpit.

---

## Track A — Foundation, contracts & rigor

### Phase 1 — Vol math + screen framework + CLI demo ✅ *(verified via `cargo xtask ci`)*
- [x] Pure vol math in `vol` — realized vol, IV rank/percentile, implied−realized
  spread, big-move detection. Deterministic, dependency-free, offline-testable.
- [x] The cheap-vol / proven-mover screen in `signals`, built over the market-data
  trait; every metric a pure function tested against known inputs.
- [x] `exub scan` demo over a synthetic universe so the whole pipeline is visible
  end-to-end before any live data.

### Phase 2 — ~~AI layer (subagents + skills)~~ — **dropped** ✂️ *(superseded by Track D)*
The original Claude-Code-specific AI layer — a subagent squad (vol-quant,
research-analyst, risk-manager, devils-advocate) and slash-command skills
(`/vol-scan`, `/research-ticker`, `/thesis`) under `.claude/` — was **removed**
(files dropped from git; `CLAUDE.md` + `README.md` reframed). *Why:* it was
harness-specific convenience, not part of the defensible engine, and it's subsumed
by the **in-engine AI layer** — `AiProvider` adapters (Phase 16), the MCP surface
(Phase 17), and the cited thesis pipeline (Phase 18). The valuable part — the
research → thesis → risk → adversarial-review *process* — moves into the engine
there, harness-agnostic. `CLAUDE.md` remains as the operating manual.

### Phase 3 — Provider-agnostic contracts ✅ *(verified via `cargo xtask ci`)*
- [x] `exub-core`: the `Provider`/`Capability` base, a unified `ProviderError`, and
  the three seams (`MarketDataProvider`, `BrokerProvider`, `AiProvider`).
- [x] Mock/paper/echo reference impls; `market-data` + `signals` depend on the
  traits, never a concrete vendor.

### Phase 4 — Config, CI & xtask scaffolding ✅ *(verified via `cargo xtask ci`)*
The 12-factor + rigor substrate, before any real I/O.
- [x] Layered `Config` (**flags > env (`EXUB_*`) > file (TOML) > defaults**);
  adapter selection + trading mode are config, not code. `tracing` logs to
  **stderr**, stdout reserved for output.
- [x] `cargo xtask ci` — the local gate (fmt, clippy `-D warnings`, build, test,
  docs, feature powerset, `cargo-deny`) — mirrored by a GitHub Actions CI job.
- [ ] `secrecy` for keys lands with the first real adapter (Phase 7) — config
  never holds secrets, so it belongs at the adapter edge, not here.

### Phase 5 — Async seams + adapter registry 🚧 *(async + registry done; streaming pending)*
The irreversible shape decision, done while mock-only (cheap now, expensive after
a live adapter).
- [x] Make all three seams `async` (`async-trait`, driven by `tokio`); the engine
  holds `Box<dyn MarketDataProvider>` / `Box<dyn AiProvider>` for runtime
  selection, and `signals` screens over `&dyn MarketDataProvider`.
- [x] A **registry** mapping a config name → boxed adapter (`build_data_provider` /
  `build_ai_provider`); a plug-in **catalog** (`exub providers`) listing every
  data feed, AI model, coding agent, and broker with wired/planned status. Coding
  agents (Claude Code, Gemini CLI, Codex) ride the `AiProvider` seam as
  `ProviderKind::Agent` + `Capability::CodingAgent`.
- [ ] Stream screen results / answers to a `TokenSink`-style surface; `--json`
  stays atomic (lands with the AI-driven `ask` command in Phase 15/16).

### Phase 6 — Canonical schema & error hardening ✅ *(verified via `cargo xtask ci`)*
- [x] Canonical types (`Bar`, `IvSnapshot`, `CheapVolResult`, `ProviderError`) are
  `#[non_exhaustive]` with constructors, so the schema evolves additively without
  breaking downstream `match`es or struct literals.
- [x] **Decision — f64 for stats, decimal at the money edge:** prices/vols stay
  `f64` (the correct representation for the statistical vol math — log returns,
  stddev, ranks); exact decimal money is reserved for the order/broker edge
  (Phase 19), *not* the vol pipeline. Timestamps stay epoch-seconds **UTC** (market
  close); a typed time crate isn't worth the dependency yet.
- [x] `ProviderError` grown to the structured set incl. `RateLimited { retry_after }`;
  **no-panic lint** (`unwrap_used`/`expect_used` denied outside tests via workspace
  lints + `clippy.toml`) — production code is panic-free.

---

## Track B — Live market data

### Phase 7 — Polygon (`massive`) market-data provider ⬜ *(plumbing — see §0.55)*
Implement `MarketDataProvider` for real, feature-gated (`polygon-live`). On its own
this is just raw access (what Massive's MCP already offers) — its payoff is Phases
8 + 17 built on top.
- [ ] REST aggregates → `daily_bars`; options snapshot → `iv_snapshot` (ATM/30-day
  IV). Key from `POLYGON_API_KEY` (env only, `SecretString`).
- [ ] Shared HTTP module (client, timeouts, retries/backoff, `Retry-After`);
  **contract tests over recorded fixtures**; live path opt-in.

### Phase 8 — IV history pipeline + storage seam ⬜ ⭐ *(the reason to exist — §0.55)*
IV rank needs 1–3yr of trailing IV that **no snapshot call returns** — so this is
the capability the engine exists to provide, not a nice-to-have. Prioritize it
right behind the first feed.
- [ ] **Acquisition is capability-driven** (a `Capability::OptionsHistory` flag +
  an `iv_history_strategy` selector): a feed with historical options-with-IV (e.g.
  Alpha Vantage) is **backfilled** in a bounded batch; a snapshot-only feed (e.g.
  Massive's IV snapshot) is **accumulated forward**. The screen is identical either
  way — the anti-corruption layer hides which feed filled the distribution.
- [ ] Persist daily ATM IV via a `Store` trait (SQLite default, in-memory for
  tests) so `iv_rank` ranks against a real distribution; the store also caches bars
  and later feeds the journal. Grounded output cites the exact history window used.

### Phase 9 — Second feed + caching / fan-out ⬜
- [ ] A second `MarketDataProvider` (e.g. Alpha Vantage, or Tradier/Alpaca data) on
  the *same* trait — the real test of agnosticism, and the case that proves the
  capability-driven IV-history strategy (backfill vs accumulate) from Phase 8.
- [ ] A caching/fan-out provider: try primary, fall back on error, cache reads to
  respect rate limits — **essential**, not optional, for a feed like Alpha Vantage
  (free tier on the order of ~25 requests/day; you can't scan a universe without
  it). **Respect each source's redistribution terms.**

### Phase 10 — Universe & symbology ⬜
- [ ] Real universe input for `exub scan` (index constituents, sector ETFs, custom
  lists) + liquidity pre-filters; a symbology layer normalizing tickers + OCC
  option symbols across providers.

---

## Track C — Discovery / signals (the point)

### Phase 11 — Screen output & ranking ⬜
- [ ] JSON + table output for `exub scan`, sortable/filterable, with the **full
  evidence** per candidate (IV rank, realized/implied, move history) and its
  provenance — a machine-readable, cited evidence set, never a verdict.

### Phase 12 — Term structure, skew & more screens ⬜
- [ ] Extend the vol math beyond ATM (term structure, skew, front-vs-back); new
  screens as pure, tested functions over canonical data. The cheap-vol screen is
  the flagship, not the only lens for *finding* setups.

### Phase 13 — Backtest harness ⬜
- [ ] Replay historical bars + IV through the screens to measure how often "cheap
  vol on a proven mover" actually realized — entry/exit simulation, summary stats.
  Offline (mock/fixtures), no live data.

### Phase 14 — Screen evals (grounding, in CI) ⬜
- [ ] Known-answer evals for every screen/metric (verifiable inputs → asserted
  outputs) + a grounding check that a surfaced candidate is backed by the data
  cited. The honesty backstop; grows every phase.

---

## Track D — The AI discovery layer

### Phase 15 — AiProvider seam depth (tool-use loop) ⬜
- [ ] Grow `AiProvider` into a tool-use loop where the model *plans a query* and
  the **engine runs it** (fetch + compute), so the model chooses *what to look
  at*, never authors the number. Streaming, token accounting, model selection.

### Phase 16 — Real AI adapters (Claude, then others) ⬜
- [ ] Implement `AiProvider` for Anthropic (tool-use, streaming), feature-gated,
  key from `ANTHROPIC_API_KEY`; contract tests over recorded completions. A second
  model (Gemini/OpenAI/local) on the same seam proves it. Distinct from the Claude
  Code *agents*, which stay the interactive brain.

### Phase 17 — Engine ↔ agent bridge + MCP surface ⬜ ⭐ *(the reason to exist — §0.55)*
The engine becomes a **higher-order MCP**, the mirror of Massive's raw-data MCP.
- [ ] Expose the engine's discovery capabilities (scan, evaluate, backtest) as MCP
  tools the AI layer / agentic assistants (Claude Code, Gemini CLI, Codex) call —
  **their** LLM reasons about *which* trade, **our** engine screens + computes +
  **cites** the number. One MCP tool-server surface over the same engine (another
  pure view). Where Massive's MCP hands over raw bars/IV and cites nothing, ours
  returns grounded cheap-vol candidates with provenance; the `massive` MCP stays a
  possible upstream feed, not a competitor.

### Phase 18 — Thesis & decision-support pipeline as code ⬜
- [ ] A reproducible, logged **scan → research → thesis → risk-check → adversarial
  review** pipeline — the desk process the dropped Claude-Code skills once
  sketched (Phase 2), now *in the engine* and harness-agnostic: evidence in, a
  **stress-tested thesis** out, every step recorded and cited. Still support, not a
  call — the human decides.

---

## Track E — Broker & execution (human-initiated, paper-first, guarded)

*Not the product — the discovery engine is. This track exists only so a trade
**you** decided on can route to **any** broker, under the §0.6 guardrails. The
engine never places or suggests placing a trade.*

### Phase 19 — Broker seam hardening ⬜
- [ ] Grow `BrokerProvider` from the skeleton: positions, open orders,
  cancel/replace, multi-leg option orders; a `TradingMode` that is *structurally*
  impossible to flip to live without an explicit human acknowledgement.

### Phase 20 — Paper broker with real sandbox fills ⬜
- [ ] Wire the paper broker to a venue's paper endpoint (Tradier/Alpaca sandbox)
  for realistic fills/slippage — zero real-money risk; contract-tested.

### Phase 21 — Risk engine (in-code) ⬜
- [ ] Codify the risk mandate as enforced code sitting in front of every
  `place_order`: sizing, max loss, portfolio Greeks limits, earnings/liquidity
  landmines. Hard veto. Guardrail §0.6-2 made mechanical.

### Phase 22 — Live-trading gate ⬜
- [ ] The deliberate, human-only path to real orders: explicit env flag + config +
  interactive confirmation + risk sign-off, all required; a dry-run that prints
  exactly what *would* be sent. Extensive tests that live mode is unreachable by
  inference.

---

## Track F — Journal, automation & product

### Phase 23 — Journal crate ⬜
- [ ] Watchlist, trade log, thesis tracking (persisted via the Phase-8 store);
  links a scan hit → research → thesis → (paper) trade → outcome, so the
  strategy's real hit rate becomes measurable.

### Phase 24 — Scheduled premarket scan & alerts ⬜
- [ ] A scheduled routine (Claude Code cron / the `schedule` skill) that runs the
  screen premarket, diffs the watchlist, and alerts on new cheap-vol candidates.

### Phase 25 — Packaging, CI & signed releases ⬜
- [ ] Tag-triggered release building `--locked` from the committed lock, checksums,
  SBOM, **keyless-signed** (cosign/sigstore); scheduled RustSec advisory audit.
  Exercise the whole packaging/signing path early.

### Phase 26 — CLI/TUI cockpit ⬜
- [ ] Grow `exub` into a cockpit: `scan`, `providers`, `research`, `thesis`,
  `journal`, `positions`, `doctor`; optional TUI dashboard.

---

## Cross-cutting standards (apply to every phase)
- **Agnostic by trait, not by `if vendor ==`.** New provider = new crate + trait
  impl. If a phase makes the engine name a vendor, the design is wrong.
- **Finds, never recommends.** No phase adds a "buy/sell" verdict or autonomous
  action. The engine surfaces cited evidence; the human decides and acts.
- **Offline-testable core.** `vol` / `exub-core` / `signals` build and test with no
  network; live adapters hide behind features and test against fixtures.
- **Guardrails are code.** Paper-default, risk-in-the-loop, human-only live gate —
  enforced by types + tests (§0.6).
- **Decimals internally, percent at the edge.** Money/vol are decimals; format only
  at display.
- **Definition of done:** `cargo test --workspace` and `cargo clippy --workspace`
  green (via `cargo xtask ci`), with tests for the new logic. Every phase.

[`MarketDataProvider`]: ./crates/core/src/market_data.rs
[`AiProvider`]: ./crates/core/src/ai.rs
[`BrokerProvider`]: ./crates/core/src/broker.rs
