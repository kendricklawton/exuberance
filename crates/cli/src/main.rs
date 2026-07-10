//! The `agent` CLI — run a detector over text and print a cited `Verdict`.
//!
//! Configuration is layered **flags > env (`AGENT_*`) > file (TOML) > defaults** (see
//! [`config`]). `check` runs the configured detector as a **wasm artifact through the sandboxed
//! host runtime** (`agent-host`: fuel/memory/epoch bounds, no imports — P3.4); the artifact
//! resolves from `artifact_dir` (default: where `cargo xtask build-detectors` writes). Output is
//! `--format human` (default) or `--format json` (the wire contract); exit codes are unchanged
//! from the P1.5 native prototype, proving the seam: `0` clean, `1` a detection fired, `2` an
//! operational error.
#![forbid(unsafe_code)]

mod config;
mod detector;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use agent_abi::Verdict;
use agent_host::WasmDetector;
use agent_sandbox::{RunOpts, Sandbox};
use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};

use crate::config::{resolve, Config, Partial};
use crate::detector::DetectorName;

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
    /// Execute an untrusted `wasm32-wasi` module in the sandbox; its stdout/stderr are streamed
    /// and its exit code is propagated.
    Run(RunArgs),
    /// Run Python code inside the sandbox (Python compiled to wasm); prints its stdout.
    RunPython(RunPythonArgs),
}

#[derive(clap::Args)]
struct RunArgs {
    /// Path to the `wasm32-wasi` module to execute.
    module: PathBuf,
    /// Forward this process's stdin to the module.
    #[arg(long)]
    stdin: bool,
    /// Arguments passed to the module as argv (after argv[0], the program name).
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
}

#[derive(clap::Args)]
struct RunPythonArgs {
    /// Python source to run. If omitted, read the program from stdin.
    code: Option<String>,
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Output format for the Verdict.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
    /// The text to scan. If omitted, read all of stdin.
    text: Option<String>,
}

/// How the Verdict is written to stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    /// A human-readable summary — one line per finding.
    Human,
    /// The `Verdict` as JSON — the wire contract other tools parse.
    Json,
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
    /// The process exit code this outcome maps to: `0` clean · `1` findings. (Operational
    /// errors are the `Err` arm in `main` and map to `2`.) Pure and unit-tested, so the
    /// exit-code contract can't drift silently.
    fn code(self) -> u8 {
        match self {
            Outcome::Clean => 0,
            Outcome::Detected => 1,
        }
    }

    fn exit_code(self) -> ExitCode {
        ExitCode::from(self.code())
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("agent: {e:#}");
            ExitCode::from(2) // operational error
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    // Destructure so the flag values move straight into the config layer — nothing is cloned.
    let Cli {
        cmd,
        config,
        detector,
        log,
    } = Cli::parse();
    let cfg = load_config(config, detector, log)?;
    init_tracing(&cfg.log);
    match cmd {
        Cmd::Check(args) => {
            tracing::debug!(detector = %cfg.detector, "resolved config");
            Ok(run_check(&cfg, args)?.exit_code())
        }
        Cmd::Run(args) => run_module(args),
        Cmd::RunPython(args) => run_python(args),
    }
}

/// Execute a `wasm32-wasi` module in the sandbox: propagate its stdout/stderr and exit code.
/// A load/trap failure is an operational error (exit 2), never a panic.
fn run_module(args: RunArgs) -> anyhow::Result<ExitCode> {
    let sandbox = Sandbox::from_file(&args.module)
        .with_context(|| format!("loading module {}", args.module.display()))?;
    let stdin = if args.stdin {
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf)?;
        buf
    } else {
        Vec::new()
    };
    let argv0 = args
        .module
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "module".to_string());
    let mut argv = vec![argv0];
    argv.extend(args.args);

    let result = sandbox
        .run(RunOpts {
            stdin,
            args: argv,
            ..RunOpts::default()
        })
        .context("running module")?;
    emit(&result.stdout, &result.stderr)?;
    Ok(exit_code_of(result.exit_code))
}

/// Run Python source inside the sandbox by handing the bundled `python.wasm` a `-c <code>` argv.
fn run_python(args: RunPythonArgs) -> anyhow::Result<ExitCode> {
    let code = match args.code {
        Some(c) => c,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };
    let wasm = python_wasm_path()?;
    let sandbox = Sandbox::from_file(&wasm).with_context(|| {
        format!(
            "loading python runtime {} (set AGENT_PYTHON_WASM to a wasm32-wasi python build)",
            wasm.display()
        )
    })?;
    let result = sandbox
        .run(RunOpts {
            args: vec!["python".to_string(), "-c".to_string(), code],
            ..RunOpts::default()
        })
        .context("running python")?;
    emit(&result.stdout, &result.stderr)?;
    Ok(exit_code_of(result.exit_code))
}

/// Resolve the Python-in-wasm artifact: `AGENT_PYTHON_WASM`, else a repo-local default.
fn python_wasm_path() -> anyhow::Result<PathBuf> {
    if let Ok(p) = std::env::var("AGENT_PYTHON_WASM") {
        return Ok(PathBuf::from(p));
    }
    Ok(PathBuf::from("wasm/python.wasm"))
}

/// Stream a run's captured output to this process's stdout/stderr.
fn emit(stdout: &[u8], stderr: &[u8]) -> anyhow::Result<()> {
    std::io::stdout().write_all(stdout)?;
    std::io::stderr().write_all(stderr)?;
    Ok(())
}

/// Map a guest exit code into a process [`ExitCode`] (a non-`u8` code clamps to 1).
fn exit_code_of(guest: i32) -> ExitCode {
    ExitCode::from(u8::try_from(guest).unwrap_or(1))
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

    match args.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&verdict)?),
        OutputFormat::Human => render(&verdict, &text),
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
    // Parse the config name into a validated identifier once; the invariant (filesystem-safe, can't
    // escape `artifact_dir`) is then carried by the type through path resolution.
    let name: DetectorName = cfg.detector.parse()?;
    let path = artifact_path(cfg, &name);
    let detector = WasmDetector::from_file(&path)
        .with_context(|| format!("loading detector '{name}' from {}", path.display()))?;
    detector
        .detect(text)
        .with_context(|| format!("running detector '{name}'"))
}

/// Resolve the artifact path for a validated detector name: `<artifact_dir>/<name>_detector.wasm`.
/// `artifact_dir` defaults to where `cargo xtask build-detectors` writes, so a from-source build
/// works with no config; a deployment points `AGENT_ARTIFACT_DIR` (or the config file) at its
/// installed artifacts. Infallible — [`DetectorName`] already guarantees the name is a bare token
/// that cannot escape `artifact_dir`.
fn artifact_path(cfg: &Config, name: &DetectorName) -> PathBuf {
    let dir = cfg
        .artifact_dir
        .clone()
        .unwrap_or_else(default_artifact_dir);
    dir.join(format!("{}_detector.wasm", name.as_str()))
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

    // Name validation itself lives with the type (`detector::tests`); this pins that a valid name
    // resolves to the expected `<name>_detector.wasm` filename.
    #[test]
    fn artifact_path_names_the_wasm_by_detector() {
        let cfg = Config {
            detector: "mock".to_string(),
            log: "warn".to_string(),
            artifact_dir: None,
        };
        let name: DetectorName = "mock".parse().expect("valid name");
        assert!(artifact_path(&cfg, &name).ends_with("mock_detector.wasm"));
    }

    // P1.6: the exit-code contract is `0` clean · `1` findings · `2`+ error. The `2` arm lives
    // in `main`'s `Err` branch; this pins the `Outcome → code` half.
    #[test]
    fn outcome_maps_to_exit_code() {
        assert_eq!(Outcome::Clean.code(), 0);
        assert_eq!(Outcome::Detected.code(), 1);
    }
}
