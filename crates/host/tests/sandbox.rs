//! P3.1 — the wasmtime sandbox: an artifact runs and returns a cited `Verdict`, and a hostile or
//! malformed artifact is a contained, typed error (fuel, epoch, ABI conformance), never a hang.
// The `wat_detector` helper below is not a `#[test]` fn; a failure to assemble its fixture WAT
// should panic and fail the test rather than be threaded as a value.
#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use agent_host::{HostError, Limits, WasmDetector, DEFAULT_FUEL, DEFAULT_MAX_MEMORY_BYTES};

/// A minimal ABI-conformant module whose `detect` never returns — the canonical runaway. `body`
/// is spliced into `detect` so each test can pick its trap; the other exports are inert.
fn wat_detector(abi_version: i32, detect_body: &str) -> Vec<u8> {
    let src = format!(
        r#"(module
             (memory (export "memory") 1)
             (func (export "abi_version") (result i32) (i32.const {abi_version}))
             (func (export "alloc") (param i32) (result i32) (i32.const 0))
             (func (export "dealloc") (param i32 i32))
             (func (export "detect") (param i32 i32) (result i32)
               {detect_body}
               (i32.const 0)))"#
    );
    wat::parse_str(src).expect("assemble test WAT")
}

#[test]
fn runs_the_mock_artifact_and_cites_a_finding() {
    let detector = WasmDetector::from_binary(&common::mock_wasm()).expect("load mock artifact");
    let verdict = detector.detect("a badword here").expect("detect");

    assert!(verdict.fired());
    assert_eq!(verdict.findings.len(), 1);
    assert_eq!(verdict.findings[0].label, "keyword.badword");
    assert_eq!(verdict.findings[0].span.start, 2);
    assert_eq!(verdict.findings[0].span.end, 9);
    assert_eq!(verdict.provenance.detector_id, "mock");
}

#[test]
fn clean_input_through_the_sandbox_has_no_findings() {
    let detector = WasmDetector::from_binary(&common::mock_wasm()).expect("load mock artifact");
    assert!(!detector
        .detect("perfectly fine text")
        .expect("detect")
        .fired());
}

#[test]
fn fuel_bomb_is_contained_as_fuel_exhausted() {
    // An infinite loop with a small fuel budget traps quickly and deterministically.
    let wasm = wat_detector(0, "(loop $l (br $l))");
    let limits = Limits {
        fuel: 100_000,
        max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
        wall_budget: Duration::from_secs(1),
    };
    let detector = WasmDetector::with_limits(&wasm, limits).expect("load fuel bomb");
    assert!(matches!(
        detector.detect("x"),
        Err(HostError::FuelExhausted)
    ));
}

#[test]
fn runaway_hits_the_wall_clock_kill_switch() {
    // Fuel is effectively unbounded, so the epoch deadline (a tight wall-clock budget) is what
    // stops the loop — the kill switch fuel alone would not provide.
    let wasm = wat_detector(0, "(loop $l (br $l))");
    let limits = Limits {
        fuel: DEFAULT_FUEL.saturating_mul(1000),
        max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
        wall_budget: Duration::from_millis(20),
    };
    let detector = WasmDetector::with_limits(&wasm, limits).expect("load runaway");
    assert!(matches!(detector.detect("x"), Err(HostError::Timeout)));
}

#[test]
fn oversized_result_prefix_is_contained_not_allocated() {
    // A hostile `detect` writes a 4 GiB framed length prefix (0xFFFF_FFFF) at offset 0 and returns
    // a pointer to it. The host must reject it as a bad pointer — bounded by the guest's memory —
    // never allocate a host buffer on the artifact's say-so.
    let wasm = wat_detector(0, "(i32.store (i32.const 0) (i32.const -1))");
    let detector = WasmDetector::with_limits(&wasm, Limits::default()).expect("load");
    assert!(matches!(detector.detect("x"), Err(HostError::BadMemory)));
}

#[test]
fn start_function_fuel_bomb_is_contained() {
    // A module whose start function loops forever fuel-bombs during *instantiation*; it must be
    // contained as FuelExhausted (routed through the trap mapper), not an opaque runtime error.
    let wasm = wat::parse_str(
        r#"(module
             (memory (export "memory") 1)
             (start $boom)
             (func $boom (loop $l (br $l)))
             (func (export "abi_version") (result i32) (i32.const 0))
             (func (export "alloc") (param i32) (result i32) (i32.const 0))
             (func (export "dealloc") (param i32 i32))
             (func (export "detect") (param i32 i32) (result i32) (i32.const 0)))"#,
    )
    .expect("assemble test WAT");
    let limits = Limits {
        fuel: 10_000,
        max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
        wall_budget: Duration::from_secs(1),
    };
    assert!(matches!(
        WasmDetector::with_limits(&wasm, limits),
        Err(HostError::FuelExhausted)
    ));
}

#[test]
fn artifact_with_wrong_abi_version_is_rejected() {
    let wasm = wat_detector(1, "");
    assert!(matches!(
        WasmDetector::from_binary(&wasm),
        Err(HostError::AbiMismatch {
            expected: 0,
            found: 1
        })
    ));
}

#[test]
fn artifact_importing_beyond_the_abi_is_rejected() {
    // A WASI clock import is exactly what the deterministic sandbox refuses to provide: an
    // artifact that reaches for one cannot load, and the error names what it reached for.
    let wasm = wat::parse_str(
        r#"(module
             (import "wasi_snapshot_preview1" "clock_time_get"
               (func (param i32 i64 i32) (result i32)))
             (memory (export "memory") 1)
             (func (export "abi_version") (result i32) (i32.const 0))
             (func (export "alloc") (param i32) (result i32) (i32.const 0))
             (func (export "dealloc") (param i32 i32))
             (func (export "detect") (param i32 i32) (result i32) (i32.const 0)))"#,
    )
    .expect("assemble test WAT");
    match WasmDetector::from_binary(&wasm) {
        Err(HostError::ForbiddenImport(name)) => {
            assert_eq!(name, "wasi_snapshot_preview1::clock_time_get");
        }
        Err(other) => panic!("expected ForbiddenImport, got {other:?}"),
        Ok(_) => panic!("expected ForbiddenImport, but the artifact loaded"),
    }
}

#[test]
fn artifact_missing_a_required_export_is_rejected() {
    // A module with no `detect` export cannot conform.
    let wasm = wat::parse_str(
        r#"(module
             (memory (export "memory") 1)
             (func (export "abi_version") (result i32) (i32.const 0))
             (func (export "alloc") (param i32) (result i32) (i32.const 0))
             (func (export "dealloc") (param i32 i32)))"#,
    )
    .expect("assemble test WAT");
    assert!(matches!(
        WasmDetector::from_binary(&wasm),
        Err(HostError::MissingExport("detect"))
    ));
}
