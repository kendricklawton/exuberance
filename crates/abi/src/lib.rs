//! The `agent` kernel's contract crate — the one thing every host, SDK, and detector artifact
//! links against.
//!
//! It holds the canonical `Verdict` wire type (P1.3) and the frozen Detector ABI framing
//! (P1.2), plus the `Detector` trait and the keyless mock detector (P1.4). It has no host or
//! runtime dependencies and compiles to `wasm32-unknown-unknown`, so a detector artifact links
//! the exact same `Verdict` and rule code the host does — the two paths are identical by
//! construction.
//!
//! *Detects and cites; never decides* — this crate defines what a detection *is*, never what to
//! do about it.
#![forbid(unsafe_code)]

pub mod abi;
pub mod detector;
pub mod verdict;

pub use abi::{AbiError, ABI_VERSION};
#[cfg(feature = "mock")]
pub use detector::mock;
pub use detector::Detector;
pub use verdict::{Finding, Provenance, Span, Verdict};
