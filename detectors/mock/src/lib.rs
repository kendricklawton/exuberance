//! The mock detector as a wasm artifact — the core-wasm ABI shim over the shared rule.
//!
//! The detection logic lives in `agent_abi::mock::detect`; this file is *only* the ABI
//! boundary (`abi_version` / `alloc` / `dealloc` / `detect`, see `agent_abi::abi`). It is the
//! project's ONLY `unsafe`: raw-pointer marshalling of bytes in and out of the module's linear
//! memory. Everything that decides *what a detection is* stays in safe, `forbid(unsafe_code)`
//! `agent-abi`, so the wasm and native paths run identical rule + serialization code.
//!
//! The crate `deny`s unsafe; the four ABI exports each opt in with `#[allow(unsafe_code)]`, so
//! the unsafe surface is exactly these functions and nothing else.
#![deny(unsafe_code)]

use agent_abi::abi::ABI_VERSION;

/// ABI export: the ABI version this artifact was built against.
#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "C" fn abi_version() -> i32 {
    ABI_VERSION
}

/// ABI export: reserve `len` bytes in the module's linear memory, returning a pointer the host
/// writes the input into. The buffer is leaked to the host, which returns it via [`dealloc`].
#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "C" fn alloc(len: i32) -> *mut u8 {
    let cap = len.max(0) as usize;
    let mut buf = Vec::<u8>::with_capacity(cap);
    let ptr = buf.as_mut_ptr();
    core::mem::forget(buf);
    ptr
}

/// ABI export: free a buffer previously returned by [`alloc`] (or by [`detect`]).
#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "C" fn dealloc(ptr: *mut u8, len: i32) {
    let cap = len.max(0) as usize;
    // SAFETY: the host passes a `ptr`/`len` pair that came from a prior `alloc(len)` (or the
    // framed buffer returned by `detect`, whose total length the host reads from the prefix).
    // Rebuilding the `Vec` with that capacity and dropping it frees exactly that allocation.
    unsafe {
        drop(Vec::from_raw_parts(ptr, 0, cap));
    }
}

/// ABI export: run detection over the UTF-8 input at `[ptr, ptr + len)` and return a pointer to
/// a framed `[len: u32 LE][UTF-8 JSON Verdict]` buffer. The host reads the 4-byte prefix, then
/// the payload, then frees the whole buffer via [`dealloc`] with total length `4 + prefix`.
#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "C" fn detect(ptr: *const u8, len: i32) -> *mut u8 {
    let len = len.max(0) as usize;
    // SAFETY: the host wrote `len` bytes of input at `ptr` via a prior `alloc(len)`.
    let input = unsafe { core::slice::from_raw_parts(ptr, len) };
    let text = core::str::from_utf8(input).unwrap_or("");

    let verdict = agent_abi::mock::detect(text);
    // Framing is infallible for a verdict this small; on the impossible error, hand back an
    // empty framed buffer (prefix 0) so the host still reads a valid length.
    let framed = verdict.encode().unwrap_or_else(|_| vec![0, 0, 0, 0]);

    let mut boxed = framed.into_boxed_slice();
    let out = boxed.as_mut_ptr();
    core::mem::forget(boxed);
    out
}
