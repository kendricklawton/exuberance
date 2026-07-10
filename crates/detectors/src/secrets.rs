//! The **secrets** rule: single-pass detection of credentials — cloud access keys, provider
//! tokens, private-key headers — plus a conservative high-entropy fallback for keys that have no
//! fixed shape (e.g. an AWS *secret* access key). Pattern + validation + entropy, no ML, no regex
//! engine: byte scanners that keep the artifact tiny and deterministic.
//!
//! Every finding carries a `secret.<kind>` label, a confidence score, and a byte span. Output is
//! sorted by span start then label, so the verdict is stable regardless of scan order.

use agent_abi::{Finding, Provenance, Verdict, ABI_VERSION};

use crate::util::{find, push_finding};

/// Stable detector id reported in provenance.
pub const ID: &str = "secrets";
/// Detector semver reported in provenance.
pub const VERSION: &str = "0.1.0";
/// Score at or above which a hit is reported — every rule below clears this.
pub const THRESHOLD: f32 = 0.5;

/// AWS access-key-id prefixes (unique-id types that front a 20-char key). See AWS IAM docs.
const AWS_PREFIXES: &[&[u8]] = &[
    b"AKIA", b"ASIA", b"AGPA", b"AIDA", b"AROA", b"ANPA", b"ANVA", b"AIPA",
];
/// GitHub token prefixes (PAT, OAuth, user-to-server, server-to-server, refresh).
const GH_PREFIXES: &[&[u8]] = &[b"ghp_", b"gho_", b"ghu_", b"ghs_", b"ghr_"];

/// Minimum length for a token to be considered by the high-entropy fallback.
const HIGH_ENTROPY_MIN_LEN: usize = 24;
/// Minimum Shannon entropy (bits/byte) for a high-entropy hit. Conservative — english prose and
/// most identifiers sit well below this; random-looking secrets sit above it.
const HIGH_ENTROPY_MIN_BITS: f32 = 4.3;

/// Run the secrets rule over `input`, returning a cited [`Verdict`].
#[must_use]
pub fn detect(input: &str) -> Verdict {
    let bytes = input.as_bytes();
    let mut findings = Vec::new();

    // Specific, high-confidence shapes first; the entropy pass then fills in only the gaps.
    scan_prefixed(
        bytes,
        AWS_PREFIXES,
        16,
        is_upper_alnum,
        "secret.aws_access_key_id",
        0.99,
        &mut findings,
    );
    scan_prefixed(
        bytes,
        GH_PREFIXES,
        36,
        |b| b.is_ascii_alphanumeric(),
        "secret.github_pat",
        0.99,
        &mut findings,
    );
    scan_private_key(bytes, &mut findings);
    scan_high_entropy(bytes, &mut findings);

    findings.sort_by(|a, b| {
        a.span
            .start
            .cmp(&b.span.start)
            .then_with(|| a.label.cmp(&b.label))
    });
    Verdict::new(
        ABI_VERSION,
        findings,
        Provenance::new(ID, VERSION, THRESHOLD),
    )
}

/// `[A-Z0-9]` — the alphabet of an AWS access key id body.
fn is_upper_alnum(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit()
}

/// Scan for `<prefix><body_len × body_char>` tokens that stand alone (not embedded in a longer
/// alphanumeric run), emitting one finding per match.
fn scan_prefixed(
    bytes: &[u8],
    prefixes: &[&[u8]],
    body_len: usize,
    body_char: impl Fn(u8) -> bool,
    label: &str,
    score: f32,
    out: &mut Vec<Finding>,
) {
    let mut i = 0;
    while i < bytes.len() {
        // A token can't start mid-run: require a non-alphanumeric byte (or start) before it.
        let boundary_before = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if boundary_before {
            if let Some(prefix) = prefixes.iter().find(|p| bytes[i..].starts_with(p)) {
                let body_start = i + prefix.len();
                let end = body_start + body_len;
                let body_ok = end <= bytes.len()
                    && bytes[body_start..end].iter().all(|&b| body_char(b))
                    // and the token must end here — not run into more alphanumerics.
                    && (end == bytes.len() || !bytes[end].is_ascii_alphanumeric());
                if body_ok {
                    push_finding(out, i, end, label, score);
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
}

/// Detect a PEM private-key header: `-----BEGIN … PRIVATE KEY-----`.
fn scan_private_key(bytes: &[u8], out: &mut Vec<Finding>) {
    const BEGIN: &[u8] = b"-----BEGIN ";
    const TAIL: &[u8] = b"PRIVATE KEY-----";
    let mut i = 0;
    while let Some(rel) = find(&bytes[i..], BEGIN) {
        let start = i + rel;
        // The key-type words sit between BEGIN and the tail; bound the search to a short window.
        let window_end = (start + 80).min(bytes.len());
        if let Some(trel) = find(&bytes[start..window_end], TAIL) {
            let end = start + trel + TAIL.len();
            push_finding(out, start, end, "secret.private_key", 1.0);
            i = end;
        } else {
            i = start + BEGIN.len();
        }
    }
}

/// Emit a `secret.high_entropy` finding for any long, high-entropy token not already covered by a
/// specific rule — the fallback for secrets with no fixed prefix (e.g. an AWS secret access key).
fn scan_high_entropy(bytes: &[u8], out: &mut Vec<Finding>) {
    let mut i = 0;
    while i < bytes.len() {
        if !is_token_char(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_token_char(bytes[i]) {
            i += 1;
        }
        let token = &bytes[start..i];
        if token.len() < HIGH_ENTROPY_MIN_LEN {
            continue;
        }
        let entropy = shannon_entropy(token);
        if entropy < HIGH_ENTROPY_MIN_BITS {
            continue;
        }
        if overlaps(start, i, out) {
            continue;
        }
        // Score scales with entropy toward the ~6 bits/byte ceiling of base64-ish text.
        let score = (entropy / 6.0).clamp(0.0, 1.0);
        push_finding(out, start, i, "secret.high_entropy", score);
    }
}

/// A byte that can appear in a base64/hex/token secret.
fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'_' | b'-')
}

/// Whether `[start, end)` overlaps any finding already recorded.
fn overlaps(start: usize, end: usize, findings: &[Finding]) -> bool {
    let (s, e) = (start as u32, end as u32);
    findings.iter().any(|f| s < f.span.end && f.span.start < e)
}

/// Shannon entropy of `bytes` in bits per byte.
fn shannon_entropy(bytes: &[u8]) -> f32 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let len = bytes.len() as f32;
    let mut h = 0.0f32;
    for &c in counts.iter() {
        if c > 0 {
            let p = c as f32 / len;
            h -= p * p.log2();
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_abi::Span;

    // Synthetic fixtures only — AWS's own public documentation example key, never a real secret.
    const AWS_EXAMPLE: &str = "AKIAIOSFODNN7EXAMPLE";

    fn labels(v: &Verdict) -> Vec<&str> {
        v.findings.iter().map(|f| f.label.as_str()).collect()
    }

    #[test]
    fn flags_an_aws_access_key_id_with_its_span() {
        let text = format!("key = {AWS_EXAMPLE} done");
        let v = detect(&text);
        let aws: Vec<_> = v
            .findings
            .iter()
            .filter(|f| f.label == "secret.aws_access_key_id")
            .collect();
        assert_eq!(aws.len(), 1);
        assert_eq!(aws[0].span, Span::new(6, 6 + 20));
        assert!(v.fired());
    }

    #[test]
    fn does_not_flag_an_aws_prefix_embedded_in_a_longer_run() {
        // 20 chars of the right shape but part of a longer alphanumeric blob → not a standalone key.
        let v = detect("XAKIAIOSFODNN7EXAMPLEX0000");
        assert!(!labels(&v).contains(&"secret.aws_access_key_id"));
    }

    #[test]
    fn flags_a_github_token() {
        let text = "token ghp_0123456789ABCDEFabcdef0123456789ABCD end"; // ghp_ + 36
        let v = detect(text);
        assert!(labels(&v).contains(&"secret.github_pat"));
    }

    #[test]
    fn flags_a_private_key_header() {
        let v = detect("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n");
        assert!(labels(&v).contains(&"secret.private_key"));
    }

    #[test]
    fn flags_a_high_entropy_secret_without_a_prefix() {
        // A base64-ish 40-char blob (an AWS secret-key shape) has no fixed prefix; entropy catches it.
        let v = detect("secret wJalrXUtnFEMIK7MDENGbPxRfiCYzEXAMPLEKEY here");
        assert!(labels(&v).contains(&"secret.high_entropy"));
    }

    #[test]
    fn clean_prose_has_no_findings() {
        let v = detect("the quick brown fox jumps over the lazy dog");
        assert!(!v.fired());
    }

    #[test]
    fn is_deterministic_and_sorted_by_span() {
        let text = format!("{AWS_EXAMPLE} and ghp_0123456789ABCDEFabcdef0123456789ABCD");
        let a = detect(&text);
        let b = detect(&text);
        assert_eq!(a, b);
        let starts: Vec<u32> = a.findings.iter().map(|f| f.span.start).collect();
        assert!(starts.windows(2).all(|w| w[0] <= w[1]));
    }
}
