//! The canonical [`Verdict`] — the wire type every surface (CLI, SDKs, sidecar) returns and
//! every detector artifact produces.
//!
//! It is the project's sacred contract: `#[non_exhaustive]` + explicit `snake_case` serde
//! naming means it evolves **additively only** — new fields are optional/defaulted, never
//! renamed, removed, or retyped — and a pinned round-trip test locks the JSON shape.

use serde::{Deserialize, Serialize};

use crate::abi::{self, AbiError, ABI_VERSION};

/// A half-open byte range `[start, end)` into the scanned input.
///
/// **Byte** offsets, not character offsets, so a host can redact losslessly. `u32` (not
/// `usize`) for wire stability across `wasm32` guests and 64-bit hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub struct Span {
    /// Byte offset of the first byte, inclusive.
    pub start: u32,
    /// Byte offset one past the last byte, exclusive.
    pub end: u32,
}

impl Span {
    /// Construct a span over `[start, end)`.
    #[must_use]
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

/// One thing a detector found: a labelled hit with a confidence score and where it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub struct Finding {
    /// Detector-defined label, e.g. `"keyword.badword"` or `"secret.aws_access_key"`.
    pub label: String,
    /// Confidence in `[0.0, 1.0]`.
    pub score: f32,
    /// Where in the input the finding is.
    pub span: Span,
}

impl Finding {
    /// Construct a finding.
    #[must_use]
    pub fn new(label: impl Into<String>, score: f32, span: Span) -> Self {
        Self {
            label: label.into(),
            score,
            span,
        }
    }
}

/// Where a verdict came from — enough to reproduce and audit it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub struct Provenance {
    /// Stable detector id, e.g. `"mock"`.
    pub detector_id: String,
    /// Detector semver, e.g. `"0.1.0"`.
    pub detector_version: String,
    /// The score threshold at or above which a hit is reported.
    pub threshold: f32,
    /// Hash of the detector's eval scorecard (Phase 6); `None` until evals exist.
    pub scorecard_hash: Option<String>,
}

impl Provenance {
    /// Construct provenance with no scorecard hash yet (Phase 6 fills it in).
    #[must_use]
    pub fn new(
        detector_id: impl Into<String>,
        detector_version: impl Into<String>,
        threshold: f32,
    ) -> Self {
        Self {
            detector_id: detector_id.into(),
            detector_version: detector_version.into(),
            threshold,
            scorecard_hash: None,
        }
    }
}

/// The result of running a detector over one input: cited findings plus provenance.
///
/// Empty `findings` means clean. *Detects and cites; never decides* — a `Verdict` says **what
/// was found and where**, never what to do about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub struct Verdict {
    /// The ABI version the producing detector speaks.
    pub abi_version: i32,
    /// Everything the detector found; empty ⇒ clean.
    pub findings: Vec<Finding>,
    /// Where this verdict came from.
    pub provenance: Provenance,
}

impl Verdict {
    /// Construct a verdict with an explicit ABI version.
    #[must_use]
    pub fn new(abi_version: i32, findings: Vec<Finding>, provenance: Provenance) -> Self {
        Self {
            abi_version,
            findings,
            provenance,
        }
    }

    /// A clean verdict (no findings) at the current [`ABI_VERSION`].
    #[must_use]
    pub fn clean(provenance: Provenance) -> Self {
        Self::new(ABI_VERSION, Vec::new(), provenance)
    }

    /// Whether the detector found anything — the signal a CLI/host turns into exit code `1`.
    #[must_use]
    pub fn fired(&self) -> bool {
        !self.findings.is_empty()
    }

    /// Serialize to a framed ABI buffer: `[len: u32 LE][UTF-8 JSON]`.
    ///
    /// # Errors
    /// [`AbiError`] if the serialized JSON is larger than a `u32` length prefix can describe.
    pub fn encode(&self) -> Result<Vec<u8>, AbiError> {
        abi::frame(&serde_json::to_vec(self)?)
    }

    /// Parse a `Verdict` from a framed ABI buffer.
    ///
    /// # Errors
    /// [`AbiError`] if the buffer is truncated or its payload is not a valid `Verdict`.
    pub fn decode(buf: &[u8]) -> Result<Self, AbiError> {
        Ok(serde_json::from_slice(abi::unframe(buf)?)?)
    }
}
