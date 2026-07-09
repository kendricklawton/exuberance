//! P3.4 — the seam is honest. `agent check` now runs the mock **through wasmtime**; the native
//! `agent_abi::mock` rule stays only as the test double. This golden proves the two paths return
//! **byte-identical** verdicts for the same input — so swapping the CLI onto the wasm runtime
//! changed the execution path, not the result.

mod common;

use agent_abi::{mock::MockDetector, Detector};
use agent_host::WasmDetector;

#[test]
fn wasm_and_native_mock_return_byte_identical_verdicts() {
    let wasm = WasmDetector::from_binary(&common::mock_wasm()).expect("load mock artifact");
    let native = MockDetector::new();

    // A spread of cases: clean, single hit, multiple labels, repeats, multibyte text (spans are
    // byte offsets), and empty input.
    let inputs = [
        "",
        "perfectly fine text",
        "a badword here",
        "injection and a secret and a badword",
        "badword badword badword",
        "café — a secret hides after multibyte bytes",
    ];

    for input in inputs {
        let via_wasm = wasm
            .detect(input)
            .expect("wasm detect")
            .encode()
            .expect("encode wasm verdict");
        let via_native = native
            .detect(input)
            .encode()
            .expect("encode native verdict");
        assert_eq!(
            via_wasm, via_native,
            "wasm and native verdict bytes differ for {input:?}"
        );
    }
}
