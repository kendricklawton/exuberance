//! [`Sandbox`] — run an untrusted `wasm32-wasi` module in a capability-scoped, metered wasmtime
//! sandbox, and capture what it did.
//!
//! This is the detector host's containment machinery (fuel, epoch kill-switch, memory ceiling,
//! instance-per-call over a cached module) pointed at a different job: instead of a pure,
//! import-free detector, it runs *code* — so the linker exposes **capability-scoped WASI**
//! (stdio + args + a scoped clock/random), never ambient authority. **No network** is wired in,
//! and stdio is captured to memory, so a run cannot reach the host or the outside world; it can
//! only read the stdin it was handed and write bytes back.
//!
//! Every run gets a **fresh [`Store`]** carrying its own fuel, memory ceiling, and epoch
//! deadline, so a hostile or buggy module is a contained [`SandboxError`] — never a hang, a
//! leak, or an escape.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use agent_host::Limits;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{I32Exit, WasiCtxBuilder};

/// How often the background thread advances the engine epoch (the wall-clock kill switch is
/// accurate to roughly one tick).
const EPOCH_TICK: Duration = Duration::from_millis(1);
/// Cap on captured stdout/stderr per run (bytes past this are dropped, not buffered unbounded).
const OUTPUT_CAP: usize = 16 * 1024 * 1024;

/// A failure loading or running a module, as a typed value — a hostile module never panics or
/// hangs the host.
#[derive(Debug)]
#[non_exhaustive]
pub enum SandboxError {
    /// The bytes are not a valid wasm module, or failed to compile.
    Compile(String),
    /// The module has no `_start` export — not a WASI command module.
    MissingStart,
    /// The module burned its compute budget (fuel) — a runaway or hostile loop, contained.
    FuelExhausted,
    /// The module exceeded its wall-clock budget (epoch deadline) — the kill switch fired.
    Timeout,
    /// The module trapped (out-of-bounds, `unreachable`, a failed allocation, …).
    Trap(String),
    /// Any other wasmtime/WASI setup error.
    Runtime(String),
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxError::Compile(e) => write!(f, "module failed to compile: {e}"),
            SandboxError::MissingStart => f.write_str("module has no `_start` export"),
            SandboxError::FuelExhausted => f.write_str("module exhausted its compute budget (fuel)"),
            SandboxError::Timeout => f.write_str("module exceeded its wall-clock budget"),
            SandboxError::Trap(e) => write!(f, "module trapped: {e}"),
            SandboxError::Runtime(e) => write!(f, "sandbox runtime error: {e}"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// What a run may do: the bytes on its stdin, its argv, and the resource budget it runs under.
#[derive(Debug, Clone)]
pub struct RunOpts {
    /// Bytes handed to the module's stdin (fd 0).
    pub stdin: Vec<u8>,
    /// argv seen by the module (argv[0] is conventionally the program name).
    pub args: Vec<String>,
    /// Fuel / memory / wall-clock ceilings (reused from the detector host).
    pub limits: Limits,
}

impl Default for RunOpts {
    fn default() -> Self {
        Self {
            stdin: Vec::new(),
            args: vec!["main".to_string()],
            limits: Limits::default(),
        }
    }
}

/// The result of a run: the guest's exit code and everything it wrote, plus what it cost.
#[derive(Debug, Clone)]
pub struct RunResult {
    /// The guest exit code (`0` on a clean return or `proc_exit(0)`).
    pub exit_code: i32,
    /// Everything the module wrote to stdout (fd 1), capped at [`OUTPUT_CAP`].
    pub stdout: Vec<u8>,
    /// Everything the module wrote to stderr (fd 2), capped at [`OUTPUT_CAP`].
    pub stderr: Vec<u8>,
    /// Units of wasm fuel the run burned.
    pub fuel_used: u64,
}

/// Per-`Store` data: the WASI context the guest talks to, plus the memory limiter.
struct StoreState {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

/// Advances an [`Engine`]'s epoch on a fixed tick so epoch deadlines become a wall-clock kill
/// switch. Dropped with the [`Sandbox`]; the thread stops and joins.
struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn spawn(engine: &Engine) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let engine = engine.clone();
        let handle = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(EPOCH_TICK);
                engine.increment_epoch();
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// A loaded module, compiled once, ready to run many times (each run in a fresh store).
pub struct Sandbox {
    engine: Engine,
    module: Module,
    _epoch: EpochTicker,
}

impl std::fmt::Debug for Sandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sandbox").finish_non_exhaustive()
    }
}

impl Sandbox {
    /// Compile a module from wasm bytes.
    ///
    /// # Errors
    /// [`SandboxError::Compile`] if the bytes are not a valid module.
    pub fn from_binary(wasm: &[u8]) -> Result<Self, SandboxError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        // Deterministic execution across hosts (see the detector host for the rationale).
        config.cranelift_nan_canonicalization(true);
        config.relaxed_simd_deterministic(true);
        let engine =
            Engine::new(&config).map_err(|e| SandboxError::Runtime(format!("engine: {e}")))?;
        let module =
            Module::from_binary(&engine, wasm).map_err(|e| SandboxError::Compile(e.to_string()))?;
        let epoch = EpochTicker::spawn(&engine);
        Ok(Self {
            engine,
            module,
            _epoch: epoch,
        })
    }

    /// Read and compile a module from a file.
    ///
    /// # Errors
    /// [`SandboxError::Runtime`] if the file cannot be read; otherwise as
    /// [`from_binary`](Self::from_binary).
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, SandboxError> {
        let bytes = std::fs::read(path.as_ref())
            .map_err(|e| SandboxError::Runtime(format!("reading module: {e}")))?;
        Self::from_binary(&bytes)
    }

    /// Run the module to completion in a fresh sandbox, capturing its output.
    ///
    /// # Errors
    /// [`SandboxError::FuelExhausted`] / [`SandboxError::Timeout`] on a runaway,
    /// [`SandboxError::MissingStart`] if it isn't a command module, [`SandboxError::Trap`] on any
    /// other trap.
    pub fn run(&self, opts: RunOpts) -> Result<RunResult, SandboxError> {
        let stdout = MemoryOutputPipe::new(OUTPUT_CAP);
        let stderr = MemoryOutputPipe::new(OUTPUT_CAP);
        let mut builder = WasiCtxBuilder::new();
        builder
            .stdin(MemoryInputPipe::new(opts.stdin))
            .stdout(stdout.clone())
            .stderr(stderr.clone())
            .args(&opts.args);
        let wasi = builder.build_p1();

        let limits = StoreLimitsBuilder::new()
            .memory_size(opts.limits.max_memory_bytes)
            .instances(1)
            .memories(1)
            .build();
        let mut store = Store::new(&self.engine, StoreState { wasi, limits });
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(opts.limits.fuel)
            .map_err(|e| SandboxError::Runtime(format!("set fuel: {e}")))?;
        store.set_epoch_deadline(deadline_ticks(opts.limits.wall_budget));

        let mut linker: Linker<StoreState> = Linker::new(&self.engine);
        p1::add_to_linker_sync(&mut linker, |s: &mut StoreState| &mut s.wasi)
            .map_err(|e| SandboxError::Runtime(format!("link wasi: {e}")))?;

        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(map_setup)?;
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|_| SandboxError::MissingStart)?;

        let outcome = start.call(&mut store, ());
        let fuel_used = opts.limits.fuel.saturating_sub(store.get_fuel().unwrap_or(0));
        let exit_code = match outcome {
            Ok(()) => 0,
            // A WASI `proc_exit(code)` unwinds as an `I32Exit` — a normal exit, not a fault.
            Err(e) => match e.downcast_ref::<I32Exit>() {
                Some(I32Exit(code)) => *code,
                None => return Err(map_trap(e)),
            },
        };

        Ok(RunResult {
            exit_code,
            stdout: stdout.contents().to_vec(),
            stderr: stderr.contents().to_vec(),
            fuel_used,
        })
    }
}

/// The wall-clock budget expressed as a count of epoch ticks (at least one).
fn deadline_ticks(wall_budget: Duration) -> u64 {
    let budget_ms = wall_budget.as_millis();
    let tick_ms = EPOCH_TICK.as_millis().max(1);
    u64::try_from(budget_ms / tick_ms).unwrap_or(u64::MAX).max(1)
}

/// Map an instantiation/setup error: a trap gets the typed treatment, anything else is a genuine
/// setup failure.
fn map_setup(err: wasmtime::Error) -> SandboxError {
    if err.downcast_ref::<Trap>().is_some() {
        map_trap(err)
    } else {
        SandboxError::Runtime(format!("instantiate: {err}"))
    }
}

/// Map a guest trap to a typed [`SandboxError`] — the two budget traps get their own variants.
fn map_trap(err: wasmtime::Error) -> SandboxError {
    match err.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => SandboxError::FuelExhausted,
        Some(Trap::Interrupt) => SandboxError::Timeout,
        Some(other) => SandboxError::Trap(other.to_string()),
        None => SandboxError::Trap(err.to_string()),
    }
}
