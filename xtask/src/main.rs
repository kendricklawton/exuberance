//! `cargo xtask <cmd>` — dev orchestration for the `agent` kernel.
//!
//! `ci` runs the full local gate (fmt, clippy, build, test, docs, feature powerset, deny) — the same
//! checks, in the same order and with the same `-D warnings` bar, that `.github/workflows/ci.yml` runs,
//! stopping at the first failure. Keyless and offline: the mock detector needs no network.
//!
//! `build-detectors` compiles each `detectors/*` source to `wasm32-unknown-unknown` (the artifacts
//! aren't run until the Phase-3 wasmtime host exists; this only proves they compile). It is deliberately
//! separate from `ci` — folding artifact builds into the gate is ROADMAP P2.3.

#![forbid(unsafe_code)]

use std::process::Command;

use anyhow::Context;
use clap::{Parser, Subcommand};

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
    /// Compile every `detectors/*` source to wasm32 (proves they build; not run until Phase 3).
    BuildDetectors,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Ci => ci(),
        Cmd::BuildDetectors => build_detectors(),
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
    // Every detector source must compile to wasm — this folds the P1.4 artifact build into the
    // gate (P2.3). Goldens still run via the native path (the `agent-abi` mock tests) until
    // P3.4 has a wasmtime runtime to execute the artifact at all. Needs the wasm32 target, which
    // `rust-toolchain.toml` pins so rustup installs it locally and in CI.
    build_detectors()?;
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
