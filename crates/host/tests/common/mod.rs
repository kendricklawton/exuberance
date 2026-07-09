//! Shared test fixtures for the host runtime.
// Test-support helpers (not `#[test]` fns, so clippy's in-tests allowance doesn't reach them):
// a failed fixture build/read should panic loudly and fail the test, not be threaded as a value.
#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

/// Path to the built mock artifact, relative to the workspace root.
const MOCK_WASM: &str = "target/detectors/wasm32-unknown-unknown/release/mock_detector.wasm";

/// The workspace root (two levels up from this crate's manifest dir, `crates/host`).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// The bytes of the real mock detector artifact, building it on demand if the gate has not
/// already (so `cargo test -p agent-host` works standalone, and is a no-op under `cargo xtask ci`,
/// which builds detectors first).
pub fn mock_wasm() -> Vec<u8> {
    let root = workspace_root();
    let artifact = root.join(MOCK_WASM);
    if !artifact.is_file() {
        let status = Command::new(env!("CARGO"))
            .current_dir(&root)
            .args([
                "build",
                "--manifest-path",
                "detectors/mock/Cargo.toml",
                "--target",
                "wasm32-unknown-unknown",
                "--release",
                "--target-dir",
                "target/detectors",
            ])
            .status()
            .expect("spawn cargo to build the mock artifact");
        assert!(status.success(), "building the mock artifact failed");
    }
    std::fs::read(&artifact).expect("read the built mock artifact")
}
