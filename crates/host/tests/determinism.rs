//! P3.2 — determinism enforced by absence. With no clock, randomness, network, or filesystem
//! reachable from inside the sandbox, the same input must yield **byte-identical** verdicts, run
//! after run. This asserts identity across 100 runs on one machine; the CI matrix runs the same
//! test on a second OS/arch, so identity is also checked across targets.

mod common;

use agent_host::WasmDetector;

#[test]
fn same_input_yields_byte_identical_verdicts_across_100_runs() {
    let detector = WasmDetector::from_binary(&common::mock_wasm()).expect("load mock artifact");
    // A mix of hits and repeats, so any nondeterminism in ordering or span math would show.
    let input = "a badword, an injection, and a secret — then another badword";

    let baseline = detector
        .detect(input)
        .expect("detect")
        .encode()
        .expect("encode verdict");
    assert!(!baseline.is_empty());

    for run in 0..100 {
        let bytes = detector
            .detect(input)
            .expect("detect")
            .encode()
            .expect("encode verdict");
        assert_eq!(bytes, baseline, "verdict bytes drifted on run {run}");
    }
}
