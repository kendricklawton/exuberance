//! Pure detection rules shared by the `detectors/*` wasm artifacts.
//!
//! Each module is a single-pass, span-accurate rule that takes text and returns an
//! [`agent_abi::Verdict`]. The excluded `detectors/<name>` cdylib is a thin FFI shim that calls
//! the matching rule here — so the rule logic lives in safe, host-testable, `cargo deny`-covered
//! code, and the only `unsafe` in the project stays in each shim's ABI boundary.
//!
//! *Detects and cites; never decides* — a rule reports **what it found and where** (label, score,
//! byte span), never what to do about it. Detectors are **deterministic by construction**: pure
//! functions of the input, no clocks/randomness/IO, so the same text always yields the same
//! verdict.
#![forbid(unsafe_code)]

pub mod pii;
pub mod secrets;
mod util;
