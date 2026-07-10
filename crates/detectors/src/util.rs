//! Small shared helpers for the detection rules.

use agent_abi::{Finding, Span};

/// Push a finding, converting byte offsets to the wire's `u32` spans without silent truncation.
/// Offsets past `u32::MAX` cannot be represented in a `Span` and never occur for real inputs.
pub(crate) fn push_finding(
    out: &mut Vec<Finding>,
    start: usize,
    end: usize,
    label: &str,
    score: f32,
) {
    if let (Ok(s), Ok(e)) = (u32::try_from(start), u32::try_from(end)) {
        out.push(Finding::new(label, score, Span::new(s, e)));
    }
}

/// First index of `needle` in `haystack` (empty needle → `Some(0)`).
pub(crate) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
