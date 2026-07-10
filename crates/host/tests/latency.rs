//! Instance-lifecycle budgets, enforced by the gate as **generous absolute ceilings**.
//!
//! Never run-to-run diffs: shared CI runners make relative perf comparisons flaky, so these are
//! fixed thresholds with orders of magnitude of headroom. A breach is a real regression — the
//! module cache lost (recompiling per call), or per-call instantiation gone pathological — not
//! noise. The numbers are recorded here and cited in `ARCHITECTURE.md` decision 002.
//!
//! Cold start and per-call are measured in **one** test so the two heavy loops don't run
//! concurrently and inflate each other; cold start is bounded by the *minimum* compile time (the
//! true lower bound on the work, robust to scheduler noise), per-call by the p99 tail.

mod common;

use std::time::{Duration, Instant};

use agent_host::WasmDetector;

/// Cold start = compiling one artifact. Cranelift compiling the ~50 KB mock (in the gate's debug
/// build) is well under a second; the ceiling leaves ample headroom, so only a systemic blowup
/// (e.g. compiling on every call) trips it.
const COLD_START_BUDGET: Duration = Duration::from_secs(2);
/// Per-call `detect` on an already-loaded detector. Instance-per-call over a *cached* module is
/// microseconds; a millisecond ceiling catches a lost module cache or pathological instantiation
/// while never flaking on genuine sub-millisecond latency.
const PER_CALL_P99_BUDGET: Duration = Duration::from_millis(10);

/// The minimum sample — the least-contended, truest measure of the underlying cost.
fn min(samples: &[Duration]) -> Duration {
    samples.iter().copied().min().unwrap_or_default()
}

/// The 99th-percentile sample (conservative: rounds up toward the max on small n).
fn p99(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    let idx = ((samples.len() * 99) / 100).min(samples.len().saturating_sub(1));
    samples[idx]
}

#[test]
fn lifecycle_budgets_are_met() {
    let wasm = common::mock_wasm();

    // Cold start: each load compiles the module once. Bound by the fastest of a few.
    let mut cold = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let detector = WasmDetector::from_binary(&wasm).expect("load artifact");
        cold.push(start.elapsed());
        drop(detector);
    }
    let cold_start = min(&cold);
    assert!(
        cold_start <= COLD_START_BUDGET,
        "cold start {cold_start:?} exceeds budget {COLD_START_BUDGET:?}"
    );

    // Per-call: instance-per-call over the cached module. Warm once, then measure a thousand.
    let detector = WasmDetector::from_binary(&wasm).expect("load artifact");
    let input = "a badword and a secret";
    let _ = detector.detect(input).expect("detect");
    let mut per_call = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let start = Instant::now();
        let _ = detector.detect(input).expect("detect");
        per_call.push(start.elapsed());
    }
    let latency = p99(per_call);
    assert!(
        latency <= PER_CALL_P99_BUDGET,
        "per-call p99 {latency:?} exceeds budget {PER_CALL_P99_BUDGET:?}"
    );
}
