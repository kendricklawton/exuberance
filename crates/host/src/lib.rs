//! The `agent` kernel's host runtime.
//!
//! This crate will embed **wasmtime** to execute detector artifacts under fuel metering, memory
//! limits, and epoch interruption, behind a deterministic import-free linker (no clocks, no
//! randomness, no network, no filesystem) — ROADMAP Phase 3.
//!
//! It is an intentional placeholder until then: the Phase-1 pipeline runs the mock detector
//! through the native `Detector` impl in `agent-abi`, so no runtime is required yet.
#![forbid(unsafe_code)]
