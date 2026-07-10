# Changelog

Notable changes to the **wire contract** (`Verdict`/`Finding` JSON + exit codes) and the
`agent` CLI. Format follows [Keep a Changelog](https://keepachangelog.com/); the project uses
[Semantic Versioning](https://semver.org/).

## Wire-contract semver policy (P1.10)

The `Verdict`/`Finding` JSON shape and the exit codes are the sacred contract, evolved
**additively only** — a change that isn't additive is a deliberate, visible major bump:

- **Additive (minor):** new *optional* fields (`#[serde(default, skip_serializing_if)]`), new
  labels, new severity buckets. An existing consumer keeps parsing unchanged.
- **Breaking (major):** renaming, removing, retyping, or reordering a field, or changing what
  an exit code means. Requires an `abi_version` bump and an entry here, and the pinned-shape
  test in `crates/abi/tests/verdict_wire.rs` must be updated in the same change.

Exit codes are contract: `0` clean · `1` findings · `2`+ operational error.

## [Unreleased]

### Added
- The host denies an over-ceiling `memory.grow` at the grow site with a typed
  `HostError::MemoryExceeded { requested_bytes, max_bytes }` (previously the guest saw a bare
  `-1` it could spin on), enforced by a custom `ResourceLimiter`; hostile huge-alloc test added.
- The wasmtime config pins determinism knobs (`cranelift_nan_canonicalization`,
  `relaxed_simd_deterministic`) so cross-arch byte-identity holds by construction for future
  float/SIMD detectors.
- `Finding` carries optional `line`, `col`, and a `redacted` preview — populated by the
  scanner, omitted from JSON when unset, so the shape stays additive.
- `Severity` (`info` · `low` · `medium` · `high` · `critical`), derived from a finding's score
  via `Severity::from_score`; a render/triage convenience, never serialized.
- `Span::slice` returns the exact bytes a span cites (bounds-checked), for previews and redaction.

### Changed
- Project reframed from an embeddable detection *kernel* to `agent scan`, the secrets/PII/leak
  scanner, powered by the same sandboxed detector runtime (see `.rules`, `ROADMAP.md`).
