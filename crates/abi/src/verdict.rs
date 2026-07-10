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

    /// The exact bytes this span cites, or `None` if it falls outside `bytes` or is inverted.
    ///
    /// Spans are lossless byte offsets, so this is precise — the scanner uses it to build a
    /// redacted preview and to locate the finding, and a host uses it to redact losslessly.
    #[must_use]
    pub fn slice<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        let (start, end) = (self.start as usize, self.end as usize);
        if start > end {
            return None;
        }
        bytes.get(start..end)
    }
}

/// A coarse severity bucket derived from a finding's confidence score.
///
/// It is a rendering/triage convenience, **not** part of the wire type — it is always
/// recomputable from `score` via [`Severity::from_score`], so it is never serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Severity {
    /// `[0.0, 0.2)` — noise floor.
    Info,
    /// `[0.2, 0.4)`.
    Low,
    /// `[0.4, 0.6)`.
    Medium,
    /// `[0.6, 0.8)`.
    High,
    /// `[0.8, 1.0]` — high-confidence hit.
    Critical,
}

impl Severity {
    /// Map a confidence score to a bucket. Boundaries are inclusive at the bottom:
    /// `info [0,0.2) · low [0.2,0.4) · medium [0.4,0.6) · high [0.6,0.8) · critical [0.8,1.0]`.
    /// Out-of-range and `NaN` scores fall to `Info` (every comparison against `NaN` is false).
    #[must_use]
    pub fn from_score(score: f32) -> Self {
        match score {
            s if s >= 0.8 => Severity::Critical,
            s if s >= 0.6 => Severity::High,
            s if s >= 0.4 => Severity::Medium,
            s if s >= 0.2 => Severity::Low,
            _ => Severity::Info,
        }
    }
}

/// One thing a detector found: a labelled hit with a confidence score and where it is.
///
/// A detector fills `label`/`score`/`span`; the **scanner** later attaches the optional
/// `line`/`col` (computed from the byte span against the file) and a `redacted` preview. Those
/// are `skip_serializing_if = "None"`, so an unset finding's JSON is byte-identical to before
/// they existed — the contract stays additive.
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
    /// 1-based line of `span.start`; filled by the scanner (a detector sees only a byte slice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based column (byte offset within the line) of `span.start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
    /// A masked preview of the matched bytes, e.g. `"AKIA…MPLE"`; the full secret is not emitted
    /// unless the caller opts out of redaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted: Option<String>,
}

impl Finding {
    /// Construct a finding. Location and redaction start unset; the scanner attaches them.
    #[must_use]
    pub fn new(label: impl Into<String>, score: f32, span: Span) -> Self {
        Self {
            label: label.into(),
            score,
            span,
            line: None,
            col: None,
            redacted: None,
        }
    }

    /// Attach a 1-based `(line, col)` location — the scanner computes this from the byte span.
    #[must_use]
    pub fn with_location(mut self, line: u32, col: u32) -> Self {
        self.line = Some(line);
        self.col = Some(col);
        self
    }

    /// Attach a redacted preview of the matched bytes.
    #[must_use]
    pub fn with_redacted(mut self, preview: impl Into<String>) -> Self {
        self.redacted = Some(preview.into());
        self
    }

    /// This finding's [`Severity`], derived from its score.
    #[must_use]
    pub fn severity(&self) -> Severity {
        Severity::from_score(self.score)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_slice_is_exact_and_bounds_checked() {
        let bytes = b"hello world";
        assert_eq!(Span::new(0, 5).slice(bytes), Some(&b"hello"[..]));
        assert_eq!(Span::new(6, 11).slice(bytes), Some(&b"world"[..]));
        assert_eq!(Span::new(11, 11).slice(bytes), Some(&b""[..])); // empty at the end
        assert_eq!(Span::new(0, 100).slice(bytes), None); // past the end
        assert_eq!(Span::new(5, 4).slice(bytes), None); // inverted
    }

    #[test]
    fn severity_buckets_map_by_score() {
        assert_eq!(Severity::from_score(0.0), Severity::Info);
        assert_eq!(Severity::from_score(0.19), Severity::Info);
        assert_eq!(Severity::from_score(0.2), Severity::Low);
        assert_eq!(Severity::from_score(0.4), Severity::Medium);
        assert_eq!(Severity::from_score(0.6), Severity::High);
        assert_eq!(Severity::from_score(0.8), Severity::Critical);
        assert_eq!(Severity::from_score(1.0), Severity::Critical);
        assert_eq!(Severity::from_score(f32::NAN), Severity::Info);
        assert_eq!(Severity::from_score(-1.0), Severity::Info);
        assert!(Severity::Critical > Severity::Info);
        // The convenience method agrees with the pure mapping.
        assert_eq!(
            Finding::new("secret.aws", 0.95, Span::new(0, 20)).severity(),
            Severity::Critical
        );
    }

    #[test]
    fn optional_fields_are_additive_and_round_trip() {
        // Unset ⇒ omitted from JSON, so the pinned shape stays byte-stable (additive-only).
        let bare = Finding::new("secret.aws", 0.9, Span::new(0, 20));
        let json = serde_json::to_string(&bare).unwrap();
        assert!(!json.contains("line") && !json.contains("redacted"));

        // Set ⇒ present, and survives a JSON round-trip.
        let rich = bare.with_location(3, 12).with_redacted("AKIA…MPLE");
        let back: Finding = serde_json::from_str(&serde_json::to_string(&rich).unwrap()).unwrap();
        assert_eq!(back, rich);
        assert_eq!(back.line, Some(3));
    }

    #[test]
    fn encode_decode_round_trips_across_cases() {
        let mut scored = Provenance::new("secrets", "0.2.0", 0.5);
        scored.scorecard_hash = Some("deadbeef".into());
        let cases = [
            Verdict::clean(Provenance::new("mock", "0.1.0", 0.5)),
            Verdict::new(
                ABI_VERSION,
                vec![
                    Finding::new("secret.aws_access_key_id", 1.0, Span::new(0, u32::MAX))
                        .with_location(1, 1)
                        .with_redacted("AKIA…MPLE"),
                    Finding::new("pii.email", 0.42, Span::new(7, 20)),
                    Finding::new("emoji.\u{1f600}", 0.0, Span::new(3, 3)),
                ],
                scored,
            ),
        ];
        for v in cases {
            assert_eq!(Verdict::decode(&v.encode().unwrap()).unwrap(), v);
        }
    }
}
