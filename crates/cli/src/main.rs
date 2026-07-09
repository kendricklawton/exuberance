//! The `agent` CLI — run a detector over text and print a cited `Verdict`.
//!
//! Configuration is layered **flags > env (`AGENT_*`) > file (TOML) > defaults** (see
//! [`config`]). `check` runs the configured detector as a **wasm artifact through the sandboxed
//! host runtime** (`agent-host`: fuel/memory/epoch bounds, no imports — P3.4); the artifact
//! resolves from `artifact_dir` (default: where `cargo xtask build-detectors` writes). The
//! surface — flags, rendering, exit codes — is unchanged from the P1.5 native prototype, proving
//! the seam: `0` clean, `1` a detection fired, `2` an operational error.
#![forbid(unsafe_code)]

mod config;

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use agent_abi::Verdict;
use agent_host::WasmDetector;
use anyhow::Context;
use clap::{Parser, Subcommand};

use crate::config::{resolve, Config, Partial};

#[derive(Parser)]
#[command(
    name = "agent",
    about = "guardrail detection — detects and cites, never decides"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// TOML config file. Precedence: flags > env (`AGENT_*`) > this file > defaults.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Detector to run (overrides `AGENT_DETECTOR` and the config file).
    #[arg(long, global = true, value_name = "NAME")]
    detector: Option<String>,

    /// Log filter for stderr, e.g. `warn`, `debug`, `agent=debug` (overrides `AGENT_LOG`).
    #[arg(long, global = true, value_name = "FILTER")]
    log: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a detector over text (an argument, or stdin) and print a cited Verdict.
    Check(CheckArgs),
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Emit the Verdict as JSON on stdout (the wire contract) instead of a human summary.
    #[arg(long)]
    json: bool,
    /// The text to scan. If omitted, read all of stdin.
    text: Option<String>,
}

/// The outcome of a check, mapped to the process exit code — avoids boolean blindness at the
/// exit boundary. (Operational errors are the `Err` arm and map to exit 2.)
enum Outcome {
    /// No findings.
    Clean,
    /// The detector fired.
    Detected,
}

impl Outcome {
    fn exit_code(self) -> ExitCode {
        match self {
            Outcome::Clean => ExitCode::SUCCESS,    // 0
            Outcome::Detected => ExitCode::from(1), // a detection fired
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(outcome) => outcome.exit_code(),
        Err(e) => {
            eprintln!("agent: {e:#}");
            ExitCode::from(2) // operational error
        }
    }
}

fn run() -> anyhow::Result<Outcome> {
    // Destructure so the flag values move straight into the config layer — nothing is cloned.
    let Cli {
        cmd,
        config,
        detector,
        log,
    } = Cli::parse();
    let cfg = load_config(config, detector, log)?;
    init_tracing(&cfg.log);
    tracing::debug!(detector = %cfg.detector, log = %cfg.log, "resolved config");
    match cmd {
        Cmd::Check(args) => run_check(&cfg, args),
    }
}

/// Initialize stderr logging, filtered by the resolved log directive. **stdout stays reserved
/// for verdicts**, so `agent check … 2>/dev/null` is pipe-clean. An invalid filter falls back to
/// `warn` rather than failing the run (a bad log setting must never break detection).
fn init_tracing(filter: &str) {
    let env_filter = tracing_subscriber::EnvFilter::try_new(filter)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .with_target(false)
        .try_init();
}

/// Fold the config layers: flags > env (`AGENT_*`) > `--config` file > defaults. Takes the flag
/// values by value so nothing is cloned.
fn load_config(
    config: Option<PathBuf>,
    detector: Option<String>,
    log: Option<String>,
) -> anyhow::Result<Config> {
    let file = match config {
        Some(path) => Partial::from_toml_file(&path)?,
        None => Partial::default(),
    };
    let env = Partial::from_env();
    let flags = Partial {
        detector,
        log,
        artifact_dir: None,
    };
    Ok(resolve(file, env, flags))
}

/// Run `agent check`, returning the [`Outcome`] that maps to the exit code.
fn run_check(cfg: &Config, args: CheckArgs) -> anyhow::Result<Outcome> {
    let text = match args.text {
        Some(t) => t,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };

    tracing::info!(detector = %cfg.detector, bytes = text.len(), "running check");
    let verdict = detect(cfg, &text)?;
    tracing::debug!(
        findings = verdict.findings.len(),
        fired = verdict.fired(),
        "verdict"
    );

    if args.json {
        println!("{}", serde_json::to_string_pretty(&verdict)?);
    } else {
        render(&verdict, &text);
    }
    Ok(if verdict.fired() {
        Outcome::Detected
    } else {
        Outcome::Clean
    })
}

/// Load the configured detector's wasm artifact and run it through the sandboxed host runtime
/// (fuel/memory/epoch bounds, no imports — P3.1/P3.2). A missing artifact or a trapped run is an
/// operational error (exit 2), never a panic. The native `agent_abi::mock` rule is no longer on
/// this path — it survives only as the test double the wasm run is checked against (P3.4 golden).
fn detect(cfg: &Config, text: &str) -> anyhow::Result<Verdict> {
    let path = artifact_path(cfg)?;
    let detector = WasmDetector::from_file(&path).with_context(|| {
        format!(
            "loading detector '{}' from {}",
            cfg.detector,
            path.display()
        )
    })?;
    detector
        .detect(text)
        .with_context(|| format!("running detector '{}'", cfg.detector))
}

/// Resolve the artifact path for the configured detector: `<artifact_dir>/<name>_detector.wasm`.
/// `artifact_dir` defaults to where `cargo xtask build-detectors` writes, so a from-source build
/// works with no config; a deployment points `AGENT_ARTIFACT_DIR` (or the config file) at its
/// installed artifacts.
///
/// The detector name becomes part of a filename, so it must be a bare identifier — this rejects
/// `/`, `.`, `..`, and the like, so a config value can't escape `artifact_dir` (`../…`) or select
/// an artifact the operator didn't mean.
fn artifact_path(cfg: &Config) -> anyhow::Result<PathBuf> {
    let name_ok = !cfg.detector.is_empty()
        && cfg
            .detector
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    anyhow::ensure!(
        name_ok,
        "invalid detector name '{}': use letters, digits, '_', or '-'",
        cfg.detector
    );
    let dir = cfg
        .artifact_dir
        .clone()
        .unwrap_or_else(default_artifact_dir);
    Ok(dir.join(format!("{}_detector.wasm", cfg.detector)))
}

/// Where `cargo xtask build-detectors` writes release artifacts, relative to the workspace root.
fn default_artifact_dir() -> PathBuf {
    PathBuf::from("target/detectors/wasm32-unknown-unknown/release")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(detector: &str) -> Config {
        Config {
            detector: detector.to_string(),
            log: "warn".to_string(),
            artifact_dir: None,
        }
    }

    #[test]
    fn rejects_detector_names_that_escape_the_artifact_dir() {
        for bad in ["../evil", "a/b", "..", ".", "mock.wasm", "", "a b"] {
            assert!(
                artifact_path(&cfg(bad)).is_err(),
                "should reject detector name {bad:?}"
            );
        }
    }

    #[test]
    fn accepts_bare_identifier_detector_names() {
        let path = artifact_path(&cfg("mock")).expect("mock is a valid name");
        assert!(path.ends_with("mock_detector.wasm"));
        assert!(artifact_path(&cfg("secrets-v2")).is_ok());
        assert!(artifact_path(&cfg("pii_us")).is_ok());
    }
}
