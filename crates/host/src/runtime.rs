//! [`WasmDetector`] — a compiled artifact, run under a fuel/memory/epoch sandbox.
//!
//! One [`WasmDetector`] owns an [`Engine`] and a compiled [`Module`] (compile once, run many).
//! Each [`detect`](WasmDetector::detect) call gets a **fresh [`Store`]** — a clean linear memory
//! with its own fuel, memory ceiling, and epoch deadline — so no state leaks between calls and a
//! runaway call cannot starve the next (instance-per-call; P3.3 measures whether to pool).
//!
//! Three bounds ride every instantiation, so a hostile or buggy artifact is a contained `Err`,
//! never a hang or a leak:
//! - **fuel** — a hard cap on executed wasm, so an infinite loop traps deterministically;
//! - **memory** — a linear-memory ceiling enforced by a [`StoreLimits`] limiter;
//! - **epoch** — a wall-clock kill switch: a background thread ticks the engine's epoch and each
//!   store trips after its deadline, catching anything fuel alone wouldn't.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use agent_abi::abi::{exports, LEN_PREFIX_BYTES};
use agent_abi::{Verdict, ABI_VERSION};
use wasmtime::{
    Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, Trap,
    TypedFunc,
};

use crate::error::HostError;

/// Default compute cap: one billion units of fuel. Generous — the reference detectors spend a
/// few thousand on a typical input; a loop that reaches this is a runaway and traps.
pub const DEFAULT_FUEL: u64 = 1_000_000_000;
/// Default linear-memory ceiling: 64 MiB. Ample for a detector's buffers; a hostile artifact
/// that tries to grow past it fails the allocation rather than exhausting the host.
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
/// Default wall-clock budget per call: the epoch kill switch fires after this even if fuel is not
/// spent (e.g. a host trap loop). Generous; a real call finishes in microseconds.
pub const DEFAULT_WALL_BUDGET: Duration = Duration::from_secs(1);
/// How often the background thread advances the engine epoch. The wall-clock kill switch is
/// accurate to roughly one tick.
const EPOCH_TICK: Duration = Duration::from_millis(1);

/// The per-call resource budget. Absolute, generous ceilings — not tuning knobs; they exist so a
/// bad artifact is *contained*, and a real call never approaches them.
///
/// `#[non_exhaustive]`: build from [`Limits::default`] and the `with_*` setters rather than a
/// struct literal, so new bounds can be added without breaking embedders (this is public SDK
/// surface at ROADMAP P7.1). Fields stay readable.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Limits {
    /// Units of wasm fuel a single `detect` may burn before it traps.
    pub fuel: u64,
    /// Maximum linear memory, in bytes, a single call may occupy.
    pub max_memory_bytes: usize,
    /// Wall-clock time a single call may run before the epoch kill switch fires.
    pub wall_budget: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: DEFAULT_FUEL,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            wall_budget: DEFAULT_WALL_BUDGET,
        }
    }
}

impl Limits {
    /// Set the fuel budget (units of wasm a single call may burn).
    #[must_use]
    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = fuel;
        self
    }

    /// Set the linear-memory ceiling in bytes.
    #[must_use]
    pub fn with_max_memory_bytes(mut self, bytes: usize) -> Self {
        self.max_memory_bytes = bytes;
        self
    }

    /// Set the wall-clock budget per call.
    #[must_use]
    pub fn with_wall_budget(mut self, budget: Duration) -> Self {
        self.wall_budget = budget;
        self
    }

    /// Reject a budget that would trap or fail *every* call, so the mistake surfaces once at load
    /// as a clear [`HostError::InvalidLimits`] rather than as a confusing per-call trap.
    fn validate(&self) -> Result<(), HostError> {
        if self.fuel == 0 {
            return Err(HostError::InvalidLimits("fuel must be non-zero"));
        }
        if self.max_memory_bytes == 0 {
            return Err(HostError::InvalidLimits(
                "max_memory_bytes must be non-zero",
            ));
        }
        Ok(())
    }
}

/// Per-`Store` data: the memory limiter the store consults on every `memory.grow`.
struct StoreState {
    limits: StoreLimits,
}

/// A background thread that advances an [`Engine`]'s epoch on a fixed tick, so epoch deadlines
/// become a wall-clock kill switch. Dropped with the [`WasmDetector`]; the thread stops and joins.
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
            // A stopped ticker thread only sleeps out its final tick; a join error means the
            // thread panicked, which the sandbox has nothing to do about — drop is infallible.
            let _ = handle.join();
        }
    }
}

/// A loaded detector artifact, ready to run under the sandbox.
pub struct WasmDetector {
    engine: Engine,
    module: Module,
    limits: Limits,
    // Kept alive for the detector's lifetime so epoch deadlines actually fire; dropped last.
    _epoch: EpochTicker,
}

impl std::fmt::Debug for WasmDetector {
    // The `Engine`/`Module` internals aren't usefully printable; surface the budget instead.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmDetector")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl WasmDetector {
    /// Compile and load an artifact from wasm bytes with the default [`Limits`], verifying it
    /// exports a conformant, version-matching ABI.
    ///
    /// # Errors
    /// [`HostError::Compile`] if the bytes are not a valid module; [`HostError::AbiMismatch`] or
    /// [`HostError::MissingExport`] if the ABI contract is not met.
    pub fn from_binary(wasm: &[u8]) -> Result<Self, HostError> {
        Self::with_limits(wasm, Limits::default())
    }

    /// Read, compile, and load an artifact from a file with the default [`Limits`].
    ///
    /// # Errors
    /// [`HostError::Runtime`] if the file cannot be read; otherwise as [`from_binary`](Self::from_binary).
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, HostError> {
        let bytes = std::fs::read(path.as_ref())
            .map_err(|e| HostError::Runtime(format!("reading artifact: {e}")))?;
        Self::from_binary(&bytes)
    }

    /// Compile and load an artifact with explicit [`Limits`].
    ///
    /// # Errors
    /// As [`from_binary`](Self::from_binary).
    pub fn with_limits(wasm: &[u8], limits: Limits) -> Result<Self, HostError> {
        limits.validate()?;
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine =
            Engine::new(&config).map_err(|e| HostError::Runtime(format!("engine: {e}")))?;
        let module =
            Module::from_binary(&engine, wasm).map_err(|e| HostError::Compile(e.to_string()))?;
        // Deterministic by absence: the linker provides *nothing* beyond the ABI, so a detector
        // cannot read a clock, draw randomness, or touch the network/filesystem — the imports do
        // not exist. Reject an artifact that reaches for any of them at load, naming the offending
        // import, rather than letting it surface as a confusing instantiate-time link failure.
        if let Some(import) = module.imports().next() {
            return Err(HostError::ForbiddenImport {
                module: import.module().to_string(),
                name: import.name().to_string(),
            });
        }
        let epoch = EpochTicker::spawn(&engine);
        let detector = Self {
            engine,
            module,
            limits,
            _epoch: epoch,
        };
        detector.verify_abi()?;
        Ok(detector)
    }

    /// Run detection over `input` in a fresh sandbox and decode the framed [`Verdict`].
    ///
    /// # Errors
    /// A trap ([`HostError::FuelExhausted`], [`HostError::Timeout`], [`HostError::Trap`]), a bad
    /// pointer ([`HostError::BadMemory`]), or a result that does not decode ([`HostError::Decode`]).
    pub fn detect(&self, input: &str) -> Result<Verdict, HostError> {
        let mut store = self.new_store()?;
        let linker = Linker::new(&self.engine);
        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(map_instantiate)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or(HostError::MissingExport("memory"))?;
        let alloc = typed_func::<i32, i32>(&instance, &mut store, exports::ALLOC)?;
        let detect = typed_func::<(i32, i32), i32>(&instance, &mut store, exports::DETECT)?;

        let len = i32::try_from(input.len())
            .map_err(|_| HostError::Runtime("input exceeds i32 length".into()))?;
        let in_ptr = alloc.call(&mut store, len).map_err(map_trap)?;
        write_bytes(&memory, &mut store, in_ptr, input.as_bytes())?;
        let out_ptr = detect.call(&mut store, (in_ptr, len)).map_err(map_trap)?;

        // Instance-per-call: the whole linear memory is reclaimed when `store` drops here, so the
        // artifact's buffers need no explicit `dealloc` (its presence is still verified at load).
        read_verdict(&memory, &store, out_ptr)
    }

    /// Instantiate once at load time to confirm the artifact exports a conformant ABI at the
    /// version this host speaks — a mismatch or missing export is caught before first use.
    fn verify_abi(&self) -> Result<(), HostError> {
        let mut store = self.new_store()?;
        let linker = Linker::new(&self.engine);
        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(map_instantiate)?;

        // Every conformant artifact exports these four functions and a linear memory.
        instance
            .get_memory(&mut store, "memory")
            .ok_or(HostError::MissingExport("memory"))?;
        let abi_version = typed_func::<(), i32>(&instance, &mut store, exports::ABI_VERSION)?;
        typed_func::<i32, i32>(&instance, &mut store, exports::ALLOC)?;
        typed_func::<(i32, i32), ()>(&instance, &mut store, exports::DEALLOC)?;
        typed_func::<(i32, i32), i32>(&instance, &mut store, exports::DETECT)?;

        let found = abi_version.call(&mut store, ()).map_err(map_trap)?;
        if found != ABI_VERSION {
            return Err(HostError::AbiMismatch {
                expected: ABI_VERSION,
                found,
            });
        }
        Ok(())
    }

    /// A fresh store carrying this detector's memory limiter, fuel, and epoch deadline.
    fn new_store(&self) -> Result<Store<StoreState>, HostError> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.max_memory_bytes)
            .instances(1)
            .memories(1)
            .build();
        let mut store = Store::new(&self.engine, StoreState { limits });
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.limits.fuel)
            .map_err(|e| HostError::Runtime(format!("set fuel: {e}")))?;
        store.set_epoch_deadline(self.deadline_ticks());
        Ok(store)
    }

    /// The wall-clock budget expressed as a count of epoch ticks (at least one).
    fn deadline_ticks(&self) -> u64 {
        let budget_ms = self.limits.wall_budget.as_millis();
        let tick_ms = EPOCH_TICK.as_millis().max(1);
        u64::try_from(budget_ms / tick_ms)
            .unwrap_or(u64::MAX)
            .max(1)
    }
}

/// Fetch a typed export, mapping absence or a signature mismatch to [`HostError::MissingExport`].
fn typed_func<Params, Results>(
    instance: &Instance,
    store: &mut Store<StoreState>,
    name: &'static str,
) -> Result<TypedFunc<Params, Results>, HostError>
where
    Params: wasmtime::WasmParams,
    Results: wasmtime::WasmResults,
{
    instance
        .get_typed_func::<Params, Results>(store, name)
        .map_err(|_| HostError::MissingExport(name))
}

/// Write `bytes` into linear memory at `ptr`, treating an out-of-bounds pointer as a typed error.
fn write_bytes(
    memory: &Memory,
    store: &mut Store<StoreState>,
    ptr: i32,
    bytes: &[u8],
) -> Result<(), HostError> {
    let offset = usize::try_from(ptr).map_err(|_| HostError::BadMemory)?;
    memory
        .write(store, offset, bytes)
        .map_err(|_| HostError::BadMemory)
}

/// Read the framed `[len: u32 LE][JSON]` buffer at `out_ptr` and decode the [`Verdict`].
///
/// The length prefix is **guest-controlled**, so this must never allocate a host buffer sized by
/// it: instead it *borrows* the guest's linear memory and slices it. A lying prefix (e.g. a
/// hostile `0xFFFF_FFFF`) then indexes past `data.len()` — which is at most the memory ceiling —
/// and is rejected as [`HostError::BadMemory`] without the host ever allocating on the artifact's
/// say-so.
fn read_verdict(
    memory: &Memory,
    store: &Store<StoreState>,
    out_ptr: i32,
) -> Result<Verdict, HostError> {
    let base = usize::try_from(out_ptr).map_err(|_| HostError::BadMemory)?;
    let data = memory.data(store);
    let prefix_end = base
        .checked_add(LEN_PREFIX_BYTES)
        .ok_or(HostError::BadMemory)?;
    let prefix: [u8; LEN_PREFIX_BYTES] = data
        .get(base..prefix_end)
        .ok_or(HostError::BadMemory)?
        .try_into()
        .map_err(|_| HostError::BadMemory)?;
    let payload_len = u32::from_le_bytes(prefix) as usize;
    let end = prefix_end
        .checked_add(payload_len)
        .ok_or(HostError::BadMemory)?;
    let framed = data.get(base..end).ok_or(HostError::BadMemory)?;
    Verdict::decode(framed).map_err(HostError::from)
}

/// Map an instantiation error. Limits are applied to the store *before* instantiation, so a
/// hostile `(start …)` function that loops or grows without bound is contained — but it traps
/// during instantiate, so route trap errors through [`map_trap`] (preserving `FuelExhausted` /
/// `Timeout`) and treat only genuine setup/link failures as [`HostError::Runtime`].
fn map_instantiate(err: wasmtime::Error) -> HostError {
    if err.downcast_ref::<Trap>().is_some() {
        map_trap(err)
    } else {
        HostError::Runtime(format!("instantiate: {err}"))
    }
}

/// Map a wasmtime call error to a typed [`HostError`] — the two budget traps get their own
/// variants so a caller can tell "hostile loop" from "genuine bug".
fn map_trap(err: wasmtime::Error) -> HostError {
    match err.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => HostError::FuelExhausted,
        Some(Trap::Interrupt) => HostError::Timeout,
        Some(Trap::MemoryOutOfBounds) => HostError::BadMemory,
        Some(other) => HostError::Trap(other.to_string()),
        None => HostError::Trap(err.to_string()),
    }
}
