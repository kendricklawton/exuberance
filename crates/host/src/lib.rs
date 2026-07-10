//! The detector runtime — the sandboxed engine the scanner drives over every input.
//!
//! [`WasmDetector`] embeds **wasmtime** to execute detector artifacts across the frozen ABI
//! (`agent_abi::abi`), under a sandbox that makes a hostile or buggy artifact a contained
//! [`HostError`], never a hang or a leak: **fuel** metering, a **memory** ceiling, and an
//! **epoch** wall-clock kill switch on every instantiation. The linker exposes nothing
//! beyond the ABI — no clocks, no randomness, no network, no filesystem — so an artifact is
//! deterministic and cannot phone home *because the imports are not there*.
//!
//! *Detects and cites; never decides* — the host runs an artifact and returns its cited
//! [`agent_abi::Verdict`]; policy lives in the embedding context, never here.
#![forbid(unsafe_code)]

mod error;
mod runtime;

pub use error::HostError;
pub use runtime::{
    Limits, WasmDetector, DEFAULT_FUEL, DEFAULT_MAX_MEMORY_BYTES, DEFAULT_WALL_BUDGET,
};
