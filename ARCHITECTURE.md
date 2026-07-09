# Architecture decisions

The record [`ROADMAP.md`](./ROADMAP.md) §0 references: every roadmap item tagged
`(decision)` produces a dated entry here — the decision, the alternatives considered, and
the why — so the reasoning outlives the diff. Entries are append-only and numbered;
reversing one is a new entry, not an edit. (Roadmap *re-scopes* — cut phases and why — are
recorded in the roadmap's tombstones, not duplicated here. P13.2 consolidates this file
before any release.)

The prior project's decision log (the retired trading engine) was cleared with the
2026-07-08 repurpose; its entries live in git history if ever needed.

Decisions queued by the roadmap, to be recorded here as they're made:
- **P3.3** — instance lifecycle on the hot path: pooling vs instance-per-call.
- **P4.2** — PII locale scope for v0.
- **P5.1** — inference approach inside the artifact (pure-Rust linear vs compiled inference
  lib; fixed-point vs float, with the cross-host determinism requirement).
- **P10.2** — registry transport: OCI vs plain HTTPS index.
- **P12.1** — sidecar protocol surface: HTTP-only vs +gRPC.

---

## 001 — 2026-07-08 — P1.2: ABI v0 is plain core-wasm exports (not the component model)

**Roadmap item:** P1.2.

**Decision.** A detector artifact is a plain **core-wasm** module (no component model, no
WASI) exporting a fixed contract, versioned by an `abi_version` export:

```
abi_version() -> i32          // must equal ABI_VERSION (= 0) for a host to run it
alloc(len: i32) -> i32        // reserve len bytes in the module's memory, return a ptr
dealloc(ptr: i32, len: i32)   // release a buffer from alloc
detect(ptr: i32, len: i32) -> i32   // detect over UTF-8 [ptr, ptr+len); return ptr to a framed buffer
```

The result buffer is framed as `[len: u32 little-endian][len bytes of UTF-8 JSON]` — a
serialized `Verdict`. Constants and the `frame`/`unframe` helpers live in `agent-abi`'s `abi`
module; both host and guest reference them so the contract has one spelling.

**Alternative considered — the WASM component model (WIT).** Typed interfaces and generated
bindings on both sides; far more ergonomic to author against.

**Why core-wasm won — reach.** The product's wedge is *one artifact, every surface* (server,
edge, browser — ROADMAP Phase 9). As of 2026, browsers and several edge runtimes still lag
component-model support and require a `jco`-style transpile/polyfill step; core wasm plus a
hand-rolled length prefix runs on every wasm host today with no transpile. WIT's ergonomics do
not outweigh losing that portability, which is the whole reason to ship detectors as artifacts.

**Migration story for the loser.** The ABI is versioned from day one. A future component-model
ABI ships as `abi_version >= N` behind a new export set, additive to the frozen v0; hosts
negotiate on the version export, and core-wasm remains the always-available lowest common
denominator. Adopting components later is a clean, non-breaking v1 decision, not a rewrite.

**Consequences.** Data crosses the boundary as bytes, so the guest depends on `agent-abi` for
the `Verdict` type + `serde_json` and the shared framing — meaning the native (CLI) path and
the wasm (artifact) path run identical serialization code, byte-identical by construction
(verified through wasmtime in P3.4). The only `unsafe` in the project is each detector's small
FFI shim (`alloc`/`detect` raw-pointer exports); the pure detection logic and framing stay in
safe, `forbid(unsafe_code)` `agent-abi`.
