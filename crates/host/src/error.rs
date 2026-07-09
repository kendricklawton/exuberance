//! [`HostError`] — every way running an artifact can fail, as a typed value.
//!
//! The host runs *untrusted* code across a trust boundary: a hostile or buggy artifact must
//! surface as a contained `Err`, never a panic, hang, or leak. So there is no `unwrap` here — a
//! bad artifact, an exhausted budget, or an out-of-bounds pointer is a variant below.

use agent_abi::AbiError;

/// A failure loading or running a detector artifact.
#[derive(Debug)]
#[non_exhaustive]
pub enum HostError {
    /// The bytes are not a valid wasm module, or failed to compile.
    Compile(String),
    /// The artifact's `abi_version` export disagrees with the host's [`agent_abi::ABI_VERSION`];
    /// running it would misread the wire.
    AbiMismatch {
        /// The version the host speaks.
        expected: i32,
        /// The version the artifact reported.
        found: i32,
    },
    /// The artifact imports something beyond the ABI (a WASI clock, randomness, the network …).
    /// The deterministic linker provides nothing, so such an artifact cannot load — the module and
    /// field are kept separate so the operator sees exactly *what* it reached for.
    ForbiddenImport {
        /// The import module, e.g. `wasi_snapshot_preview1`.
        module: String,
        /// The import field, e.g. `clock_time_get`.
        name: String,
    },
    /// The requested [`crate::Limits`] are unusable (e.g. zero fuel or zero memory would trap or
    /// fail every call); rejected at load rather than surfacing as a confusing per-call trap.
    InvalidLimits(&'static str),
    /// A required ABI export (`abi_version` / `alloc` / `dealloc` / `detect`, or `memory`) is
    /// missing or has the wrong signature.
    MissingExport(&'static str),
    /// The artifact burned its compute budget (fuel) — a runaway or hostile loop, contained.
    FuelExhausted,
    /// The artifact exceeded its wall-clock budget (epoch deadline) — the kill switch fired.
    Timeout,
    /// The artifact trapped for some other reason (out-of-bounds access, `unreachable`, …).
    Trap(String),
    /// The artifact handed back a pointer/length outside its own linear memory.
    BadMemory,
    /// The framed result buffer did not decode to a [`agent_abi::Verdict`].
    Decode(AbiError),
    /// Any other wasmtime error (instantiation, a host call) not covered above.
    Runtime(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Compile(e) => write!(f, "artifact failed to compile: {e}"),
            HostError::AbiMismatch { expected, found } => write!(
                f,
                "artifact ABI version {found} does not match host ABI version {expected}"
            ),
            HostError::ForbiddenImport { module, name } => write!(
                f,
                "artifact imports `{module}::{name}`, which the deterministic sandbox does not provide"
            ),
            HostError::InvalidLimits(why) => write!(f, "invalid sandbox limits: {why}"),
            HostError::MissingExport(name) => {
                write!(f, "artifact is missing the required `{name}` export")
            }
            HostError::FuelExhausted => f.write_str("artifact exhausted its compute budget (fuel)"),
            HostError::Timeout => f.write_str("artifact exceeded its wall-clock budget"),
            HostError::Trap(e) => write!(f, "artifact trapped: {e}"),
            HostError::BadMemory => f.write_str("artifact returned a pointer outside its memory"),
            HostError::Decode(e) => write!(f, "artifact result did not decode: {e}"),
            HostError::Runtime(e) => write!(f, "host runtime error: {e}"),
        }
    }
}

impl std::error::Error for HostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HostError::Decode(e) => Some(e),
            _ => None,
        }
    }
}

impl From<AbiError> for HostError {
    fn from(e: AbiError) -> Self {
        HostError::Decode(e)
    }
}
