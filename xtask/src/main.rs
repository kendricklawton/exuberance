//! `cargo xtask <cmd>` — dev orchestration for the exuberance engine.
//!
//! `ci` runs the full local gate (fmt, clippy, build, test, docs, feature powerset, deny) — the same
//! checks, in the same order and with the same `-D warnings` bar, that `.github/workflows/ci.yml` runs,
//! stopping at the first failure. No API keys needed: tests drive the mock adapters.

#![forbid(unsafe_code)]

use std::process::Command;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "dev orchestration for exuberance")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the full local gate (fmt, clippy, build, test, docs, feature powerset, deny) — mirrors CI.
    Ci,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Ci => ci(),
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
    println!("\n\u{2713} all checks passed");
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
