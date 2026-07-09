//! The `agent` CLI — run a detector over text and print a cited `Verdict`.
//!
//! Phase 1 wires the `check` subcommand to the **native** mock detector — no wasmtime yet. The
//! Phase-3 host swaps in a `WasmDetector` behind the same `agent_abi::Detector` trait, and this
//! surface doesn't change. Exit codes are part of the wire contract: `0` clean, `1` a detection
//! fired, `2` an operational error.
#![forbid(unsafe_code)]

use std::io::Read;
use std::process::ExitCode;

use agent_abi::{Detector, Verdict};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "agent",
    about = "guardrail detection — detects and cites, never decides"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a detector over text (an argument, or stdin) and print a cited Verdict.
    Check(CheckArgs),
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Which detector to run. Only the keyless `mock` exists in Phase 1.
    #[arg(long, default_value = "mock")]
    detector: String,
    /// Emit the Verdict as JSON on stdout (the wire contract) instead of a human summary.
    #[arg(long)]
    json: bool,
    /// The text to scan. If omitted, read all of stdin.
    text: Option<String>,
}

fn main() -> ExitCode {
    let result = match Cli::parse().cmd {
        Cmd::Check(args) => run_check(args),
    };
    match result {
        Ok(true) => ExitCode::from(1),  // a detection fired
        Ok(false) => ExitCode::SUCCESS, // clean
        Err(e) => {
            eprintln!("agent: {e:#}");
            ExitCode::from(2) // operational error
        }
    }
}

/// Run `agent check`. Returns `Ok(true)` if the detector fired, `Ok(false)` if clean.
fn run_check(args: CheckArgs) -> anyhow::Result<bool> {
    let text = match args.text {
        Some(t) => t,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };

    let verdict = detect(&args.detector, &text)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&verdict)?);
    } else {
        render(&verdict, &text);
    }
    Ok(verdict.fired())
}

/// Resolve a detector name to its `Detector` and run it. An unknown name is an operational
/// error (exit 2), never a panic.
fn detect(name: &str, text: &str) -> anyhow::Result<Verdict> {
    match name {
        "mock" => Ok(agent_abi::mock::MockDetector::new().detect(text)),
        other => anyhow::bail!("unknown detector '{other}' (available: mock)"),
    }
}

/// Human-readable render to stdout: a header plus one line per finding with its span, score,
/// and the matched excerpt.
fn render(verdict: &Verdict, text: &str) {
    let p = &verdict.provenance;
    if verdict.findings.is_empty() {
        println!(
            "clean — no findings ({} v{})",
            p.detector_id, p.detector_version
        );
        return;
    }
    println!(
        "{} finding(s) — {} v{}:",
        verdict.findings.len(),
        p.detector_id,
        p.detector_version
    );
    for f in &verdict.findings {
        let (start, end) = (f.span.start as usize, f.span.end as usize);
        let excerpt = text.get(start..end).unwrap_or("");
        println!(
            "  [{start}..{end}] {} (score {:.2}) \u{201c}{excerpt}\u{201d}",
            f.label, f.score
        );
    }
}
