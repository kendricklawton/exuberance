//! `cargo xtask <cmd>` — dev orchestration for the `agent` kernel.
//!
//! `ci` runs the full local gate (fmt, clippy, build, test, docs, feature powerset, deny) — the same
//! checks, in the same order and with the same `-D warnings` bar, that `.github/workflows/ci.yml` runs,
//! stopping at the first failure. Keyless and offline: the mock detector needs no network.
//!
//! `build-detectors` compiles each `detectors/*` source to `wasm32-unknown-unknown`; `goldens`
//! then runs each detector's `cases/` inputs through its built artifact (via the host runtime) and
//! asserts the returned `Verdict` matches the committed expectation. `ci` runs both after the host
//! checks, so a detector change that shifts a verdict fails the gate unless its goldens are updated.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agent_abi::Verdict;
use agent_host::WasmDetector;
use anyhow::Context;
use clap::{Parser, Subcommand};
use serde::Deserialize;

#[derive(Parser)]
#[command(name = "xtask", about = "dev orchestration for the agent kernel")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the full local gate (fmt, clippy, build, test, docs, feature powerset, deny) — mirrors CI.
    Ci,
    /// Compile every `detectors/*` source to wasm32.
    BuildDetectors,
    /// Build the detectors, then run each one's `cases/` goldens against the built artifact.
    Goldens,
    /// Enforce each detector's `agent.toml` budgets (wasm size, p99 latency, declared labels)
    /// against the already-built artifact.
    Budgets,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Ci => ci(),
        Cmd::BuildDetectors => build_detectors(),
        Cmd::Goldens => {
            build_detectors()?;
            goldens()
        }
        Cmd::Budgets => budgets(),
    }
}

fn ci() -> anyhow::Result<()> {
    // `--locked` everywhere so a stale Cargo.lock fails here, not in CI (which also builds --locked).
    // The `hack` and `deny` steps assume `cargo-hack` and `cargo-deny` are installed
    // (`cargo install cargo-hack cargo-deny`), exactly as CI's toolchain provides them.
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--all-targets",
        "--all-features",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["build", "--locked"])?;
    cargo(&["test", "--all-features", "--locked"])?;
    // Docs are a first-class surface: broken/redundant intra-doc links fail here (rustdoc `-D warnings`),
    // not silently on the published docs.
    cargo_env(
        &[
            "doc",
            "--no-deps",
            "--workspace",
            "--all-features",
            "--locked",
        ],
        &[("RUSTDOCFLAGS", "-D warnings")],
    )?;
    // No --locked here: --no-dev-deps rewrites the manifests, which would force a lock update that
    // --locked forbids. Lock freshness is already gated by the build/test/clippy steps above.
    cargo(&[
        "hack",
        "--feature-powerset",
        "--no-dev-deps",
        "check",
        "--workspace",
    ])?;
    cargo(&["deny", "check"])?;
    // Compile every detector to wasm (P2.3), then run its goldens against the built artifact
    // through the host runtime (P4.3, below). Needs the wasm32 target, which `rust-toolchain.toml`
    // pins so rustup installs it locally and in CI.
    build_detectors()?;
    // Run each detector's goldens against the freshly-built artifact — a detector change that
    // shifts a verdict fails here unless its `cases/` are updated in the same change (P4.3).
    goldens()?;
    // Enforce each detector's `agent.toml` budgets against the built artifact (P4.4).
    budgets()?;
    println!("\n\u{2713} all checks passed");
    Ok(())
}

/// Build every detector under `detectors/` (each dir with a `Cargo.toml`) to
/// `wasm32-unknown-unknown`. Output goes under the git-ignored `target/detectors` so nested
/// build dirs never leak into the repo. Detectors are excluded from the workspace, so this is
/// the only place they're compiled.
fn build_detectors() -> anyhow::Result<()> {
    let mut built = 0usize;
    for entry in std::fs::read_dir("detectors")? {
        let dir = entry?.path();
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let manifest = manifest.to_str().context("detector path is not UTF-8")?;
        cargo(&[
            "build",
            "--manifest-path",
            manifest,
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--target-dir",
            "target/detectors",
        ])?;
        built += 1;
    }
    anyhow::ensure!(built > 0, "no detectors found under detectors/");
    println!("\n\u{2713} built {built} detector artifact(s) → target/detectors");
    Ok(())
}

/// Run every detector's golden cases against its **built artifact**: for each
/// `detectors/*/cases/`, load `<name>_detector.wasm` through the host runtime, run each
/// `<stem>.txt` input, and assert the returned `Verdict` equals the committed
/// `<stem>.verdict.json`. Comparison is by decoded value (whitespace-insensitive). Assumes
/// [`build_detectors`] already ran (it does in [`ci`] and the `goldens` subcommand).
fn goldens() -> anyhow::Result<()> {
    let mut checked = 0usize;
    for entry in std::fs::read_dir("detectors")? {
        let dir = entry?.path();
        let cases = dir.join("cases");
        if !cases.is_dir() {
            continue;
        }
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .context("detector dir name is not UTF-8")?;
        let artifact = artifact_wasm(name);
        let detector = WasmDetector::from_file(&artifact)
            .map_err(|e| anyhow::anyhow!("loading {}: {e}", artifact.display()))?;

        // Deterministic order so a failure is stable and the count reproducible.
        let mut inputs: Vec<PathBuf> = std::fs::read_dir(&cases)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "txt"))
            .collect();
        inputs.sort();

        for input_path in inputs {
            let expected_path = input_path.with_extension("verdict.json");
            let input = std::fs::read_to_string(&input_path)
                .with_context(|| format!("reading {}", input_path.display()))?;
            let expected_json = std::fs::read_to_string(&expected_path)
                .with_context(|| format!("reading {}", expected_path.display()))?;
            let expected: Verdict = serde_json::from_str(&expected_json)
                .with_context(|| format!("parsing {}", expected_path.display()))?;
            let actual = detector
                .detect(&input)
                .map_err(|e| anyhow::anyhow!("running {name} on {}: {e}", input_path.display()))?;
            anyhow::ensure!(
                actual == expected,
                "golden mismatch: {}\n  expected: {}\n  actual:   {}",
                input_path.display(),
                serde_json::to_string(&expected)?,
                serde_json::to_string(&actual)?,
            );
            checked += 1;
        }
    }
    anyhow::ensure!(
        checked > 0,
        "no golden cases found under detectors/*/cases/"
    );
    println!("\n\u{2713} {checked} golden case(s) passed");
    Ok(())
}

/// Path to a built detector artifact by detector-directory name.
fn artifact_wasm(name: &str) -> PathBuf {
    Path::new("target/detectors/wasm32-unknown-unknown/release")
        .join(format!("{name}_detector.wasm"))
}

/// A detector's `agent.toml`: the labels it may emit and the budgets the gate enforces.
#[derive(Deserialize)]
struct Manifest {
    id: String,
    labels: Vec<String>,
    budgets: Budgets,
}

/// Absolute ceilings enforced against the built artifact (generous, not tuning targets).
#[derive(Deserialize)]
struct Budgets {
    max_wasm_bytes: u64,
    max_p99_micros: u64,
}

/// Number of timed calls per detector for the p99 latency measurement.
const BUDGET_SAMPLES: usize = 500;

/// Enforce each detector's `agent.toml` against its built artifact: **size** (wasm ≤
/// `max_wasm_bytes`), **latency** (measured p99 ≤ `max_p99_micros`), and **label discipline** (the
/// artifact's provenance id matches, and it emits no label the manifest doesn't declare). Assumes
/// [`build_detectors`] already ran.
fn budgets() -> anyhow::Result<()> {
    let mut checked = 0usize;
    for entry in std::fs::read_dir("detectors")? {
        let dir = entry?.path();
        let manifest_path = dir.join("agent.toml");
        if !manifest_path.is_file() {
            continue;
        }
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .context("detector dir name is not UTF-8")?;
        let manifest: Manifest = toml::from_str(&std::fs::read_to_string(&manifest_path)?)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;

        let artifact = artifact_wasm(name);
        let size = std::fs::metadata(&artifact)
            .with_context(|| format!("stat {}", artifact.display()))?
            .len();
        anyhow::ensure!(
            size <= manifest.budgets.max_wasm_bytes,
            "{name}: artifact is {size} bytes, over the {} byte budget",
            manifest.budgets.max_wasm_bytes
        );

        let detector = WasmDetector::from_file(&artifact)
            .map_err(|e| anyhow::anyhow!("loading {}: {e}", artifact.display()))?;
        let sample = representative_input(&dir)?;

        // Label discipline: the artifact must own its manifest id and emit only declared labels.
        let verdict = detector
            .detect(&sample)
            .map_err(|e| anyhow::anyhow!("running {name}: {e}"))?;
        anyhow::ensure!(
            verdict.provenance.detector_id == manifest.id,
            "{name}: artifact id {:?} != manifest id {:?}",
            verdict.provenance.detector_id,
            manifest.id
        );
        for f in &verdict.findings {
            anyhow::ensure!(
                manifest.labels.iter().any(|l| l == &f.label),
                "{name}: emitted label {:?} is not declared in agent.toml",
                f.label
            );
        }

        let p99 = measure_p99(&detector, &sample, BUDGET_SAMPLES)?;
        let budget = Duration::from_micros(manifest.budgets.max_p99_micros);
        anyhow::ensure!(
            p99 <= budget,
            "{name}: p99 latency {p99:?} exceeds budget {budget:?}"
        );
        checked += 1;
    }
    anyhow::ensure!(checked > 0, "no detector manifests found under detectors/");
    println!("\n\u{2713} {checked} detector budget(s) held");
    Ok(())
}

/// A representative input for latency/label measurement: the detector's own golden case inputs
/// joined, falling back to a short default if it has none.
fn representative_input(dir: &Path) -> anyhow::Result<String> {
    let cases = dir.join("cases");
    let mut buf = String::new();
    if cases.is_dir() {
        let mut inputs: Vec<PathBuf> = std::fs::read_dir(&cases)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "txt"))
            .collect();
        inputs.sort();
        for input in inputs {
            buf.push_str(&std::fs::read_to_string(&input)?);
            buf.push('\n');
        }
    }
    if buf.is_empty() {
        buf.push_str("the quick brown fox jumps over the lazy dog");
    }
    Ok(buf)
}

/// Measure p99 per-call latency over `n` timed detections (after one warm-up call).
fn measure_p99(detector: &WasmDetector, input: &str, n: usize) -> anyhow::Result<Duration> {
    detector
        .detect(input)
        .map_err(|e| anyhow::anyhow!("warm-up detect failed: {e}"))?;
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let start = Instant::now();
        detector
            .detect(input)
            .map_err(|e| anyhow::anyhow!("detect failed: {e}"))?;
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    let idx = ((n * 99) / 100).min(n.saturating_sub(1));
    Ok(samples[idx])
}

fn cargo(args: &[&str]) -> anyhow::Result<()> {
    cargo_env(args, &[])
}

fn cargo_env(args: &[&str], env: &[(&str, &str)]) -> anyhow::Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.args(args);
    // Mirror ci.yml's workflow-level `RUSTFLAGS: -D warnings`: deny compiler warnings on *every* step
    // (build/test/powerset), not just the clippy pass — otherwise a plain rustc warning could pass here yet
    // fail CI. Callers layer more env (e.g. RUSTDOCFLAGS) on top.
    cmd.env("RUSTFLAGS", "-D warnings");
    for (key, value) in env {
        cmd.env(key, value);
    }
    let status = cmd.status()?;
    anyhow::ensure!(status.success(), "`cargo {}` failed", args.join(" "));
    Ok(())
}
