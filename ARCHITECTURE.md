# Architecture decisions

The record [`ROADMAP.md`](./ROADMAP.md) §0 references: every roadmap item tagged
`(decision)` produces a dated entry here — the decision, the alternatives considered, and
the why — so the reasoning outlives the diff. Entries are append-only and numbered;
reversing one is a new entry, not an edit. (Roadmap *re-scopes* — cut phases and why — are
recorded in the roadmap's tombstones, not duplicated here. P25.3 consolidates this file
before any release.)

---

## 001 — f64 for stats, decimal at the money edge (2026-07-08 · accepted)

**Roadmap item:** P6.2.

**Context.** The engine's numbers split into two families with different correctness
needs: *statistical* values (prices feeding log returns, standard deviations, realized
vol, IV ranks) where floating point is the standard and correct representation, and
*money* values (order prices, account balances) where exact decimal arithmetic matters
because rounding errors compound into real cents.

**Decision.**
- Prices, vols, returns, and every derived statistic stay **`f64`** end-to-end (`Bar`,
  `IvSnapshot`, the `vol` crate, `CheapVolResult`).
- **Exact decimal money belongs at an order/broker edge — which is cut by design**
  (ROADMAP Phases 19–22 tombstone: the engine places no orders). If execution is ever
  explicitly re-scoped, a decimal money type arrives with it as part of that re-scope's
  design; nothing in the discovery pipeline needs it.
- Timestamps stay **epoch-seconds UTC** (`Bar.t: i64`, market close). No typed time crate
  yet.

**Alternatives considered.**
- *`rust_decimal` (or similar) everywhere* — wrong tool for the statistical path
  (log/sqrt/stddev live in `f64`; converting at every math call adds cost and noise) and a
  pervasive dependency in crates that are deliberately dependency-free (`vol` has zero
  deps).
- *A typed time crate (`time`/`chrono`/`jiff`) now* — buys nothing while the only
  timestamps are daily-close markers from a mock. The real need appears with a live
  adapter's trading-calendar / session / timezone handling — **revisit inside Phase 7's
  data-correctness work (P7.2)**, where the cost is justified by actual session logic.

**Consequences.**
- `vol` stays pure and dependency-free; the whole discovery pipeline is `f64` with
  documented decimal semantics (0.30 == 30%; percent only at the display edge).
- `Bar.t`'s meaning (epoch seconds, UTC, market close) is documented on the type; any
  future granularity change is additive via the `#[non_exhaustive]` schema.
- Phase 7 owns the calendar/session/timezone story and is the checkpoint for the
  time-crate question.

---

## 002 — The IV-history store: seam in core, SQLite adapter, decorator acquisition (2026-07-08 · accepted)

**Roadmap item:** P8.1–P8.3 (the reason to exist — §0 keystone 3b).

**Context.** IV *rank* needs a 1–3yr trailing IV *series*; a snapshot feed returns one value
per call. The engine must persist observations and rank against the accumulated distribution.
Three sub-decisions: where the persistence seam lives, what backs it, and how acquisition
composes with the screen.

**Decision.**
- **Seam in `exub-core`, adapters outside.** `IvStore` (the port) + `MemoryIvStore` (dep-free,
  tests/lean build) live in core; the persistent adapter is a separate `crates/store`. Ports &
  adapters, and it keeps core dependency-light.
- **SQLite (`rusqlite`, `bundled`) for the persistent store**, not a flat JSON file. This same
  store is the future home of the journal (P23) and a bar cache (P9's caching theme); doing it
  once avoids a throwaway. `bundled` vendors the SQLite C amalgamation → no system dependency,
  builds fully offline. Gated behind the cli `sqlite` feature so a lean build compiles no C.
- **Acquisition is a decorator, not screen surgery.** `StoreBackedSource` wraps any feed + a
  store and, keyed on `iv_history_strategy(feed)`, **accumulates** a snapshot feed forward or
  **backfills** from a history-capable feed (a new defaulted `MarketDataProvider::iv_history`).
  It returns a canonical `IvSnapshot` with the assembled distribution + provenance — so
  `signals` is byte-for-byte unchanged (the anti-corruption layer P8.1 asks for).

**Alternatives considered.**
- *JSON-file store now* — simpler and dep-free, but thrown away the moment P23's journal wants
  relational queries; the seam would survive but the impl wouldn't. Rejected to avoid rework.
- *Store parameter threaded through `evaluate`/`scan`* — pollutes the pure screen signature
  with persistence. The decorator keeps the screen a pure function of a provider.
- *`async` SQLite (`sqlx`)* — heavier, and SQLite is inherently local/sync. `rusqlite` behind
  an `Arc<Mutex<Connection>>` with no `.await` held across the lock is correct for a
  single-user CLI and far lighter.

**Consequences.**
- The store is **opt-in** in the CLI (`--store PATH` / `EXUB_STORE_PATH`); unset → an ephemeral
  in-memory store, so the default demo stays side-effect-free and `MockSource` (which carries
  its own inline history for the demo) is untouched.
- `IvSnapshot` gained `history: Option<IvHistoryMeta>` and `CheapVolResult` gained
  `iv_history_len` / `span_days` / `source` — additive via the P6.1 discipline; grounded output
  cites the window.
- **Bar caching is deferred to P9** (its caching/fan-out theme); P8's store is IV-only.
- A real ranked distribution needs observations on *different days* — one run accumulates one
  observation, so rank stays `None` until history builds (honest, not faked).
