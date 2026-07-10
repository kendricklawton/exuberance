//! The **pii** rule: single-pass detection of personally-identifying data — emails, IPv4
//! addresses, US phone numbers, US SSNs, and payment-card numbers — validation-aware to hold
//! precision (octet ranges, SSN area rules, the Luhn checksum). Pattern + validation, no ML, no
//! regex engine: byte scanners that keep the artifact tiny and deterministic.
//!
//! **Locale scope (v0):** locale-neutral shapes (email, IPv4) plus **US** shapes (phone, SSN) and
//! checksum-validated payment cards. EU and other locales are additive later — see
//! `ARCHITECTURE.md` decision 003.

use agent_abi::{Provenance, Verdict, ABI_VERSION};

use crate::util::push_finding;

/// Stable detector id reported in provenance.
pub const ID: &str = "pii";
/// Detector semver reported in provenance.
pub const VERSION: &str = "0.1.0";
/// Score at or above which a hit is reported.
pub const THRESHOLD: f32 = 0.5;

/// Run the pii rule over `input`, returning a cited [`Verdict`].
#[must_use]
pub fn detect(input: &str) -> Verdict {
    let bytes = input.as_bytes();
    let mut findings = Vec::new();

    scan_email(bytes, &mut findings);
    scan_ipv4(bytes, &mut findings);
    scan_us_phone(bytes, &mut findings);
    scan_us_ssn(bytes, &mut findings);
    scan_credit_card(bytes, &mut findings);

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

// --- email -----------------------------------------------------------------------------------

fn is_email_local(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-')
}
fn is_email_domain(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')
}

/// Detect `local@domain.tld`, anchoring on `@` and expanding both ways.
fn scan_email(bytes: &[u8], out: &mut Vec<agent_abi::Finding>) {
    for idx in 0..bytes.len() {
        if bytes[idx] != b'@' {
            continue;
        }
        let mut start = idx;
        while start > 0 && is_email_local(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = idx + 1;
        while end < bytes.len() && is_email_domain(bytes[end]) {
            end += 1;
        }
        if start < idx && valid_domain(&bytes[idx + 1..end]) {
            push_finding(out, start, end, "pii.email", 0.9);
        }
    }
}

/// A domain with a dotted label + a ≥2-letter alphabetic TLD, no leading/trailing dot.
fn valid_domain(domain: &[u8]) -> bool {
    if domain.first() == Some(&b'.') || domain.last() == Some(&b'.') {
        return false;
    }
    match domain.iter().rposition(|&b| b == b'.') {
        Some(dot) => {
            let tld = &domain[dot + 1..];
            tld.len() >= 2 && tld.iter().all(u8::is_ascii_alphabetic)
        }
        None => false,
    }
}

// --- IPv4 ------------------------------------------------------------------------------------

/// Detect dotted-quad IPv4 with each octet in `0..=255`.
fn scan_ipv4(bytes: &[u8], out: &mut Vec<agent_abi::Finding>) {
    let mut i = 0;
    while i < bytes.len() {
        let boundary_before = i == 0 || !(bytes[i - 1].is_ascii_digit() || bytes[i - 1] == b'.');
        if boundary_before && bytes[i].is_ascii_digit() {
            if let Some(end) = parse_ipv4(bytes, i) {
                if end == bytes.len() || !(bytes[end].is_ascii_digit() || bytes[end] == b'.') {
                    push_finding(out, i, end, "pii.ipv4", 0.8);
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
}

fn parse_ipv4(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    for octet in 0..4 {
        if octet > 0 {
            if bytes.get(i) != Some(&b'.') {
                return None;
            }
            i += 1;
        }
        i = parse_octet(bytes, i)?;
    }
    Some(i)
}

/// Consume 1–3 digits worth ≤ 255, returning the index past them.
fn parse_octet(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    while j < bytes.len() && bytes[j].is_ascii_digit() && j - i < 3 {
        j += 1;
    }
    if j == i {
        return None;
    }
    let val = bytes[i..j]
        .iter()
        .fold(0u16, |a, &b| a * 10 + u16::from(b - b'0'));
    if val > 255 {
        return None;
    }
    Some(j)
}

// --- US phone --------------------------------------------------------------------------------

fn is_phone_sep(b: u8) -> bool {
    matches!(b, b'-' | b'.' | b' ')
}
fn skip_seps(bytes: &[u8], mut j: usize) -> usize {
    while j < bytes.len() && is_phone_sep(bytes[j]) {
        j += 1;
    }
    j
}
fn read_digits(bytes: &[u8], start: usize, n: usize) -> Option<usize> {
    let end = start + n;
    if end <= bytes.len() && bytes[start..end].iter().all(u8::is_ascii_digit) {
        Some(end)
    } else {
        None
    }
}

/// Detect a US/NANP phone number in `ddd-ddd-dddd` / `ddd.ddd.dddd` / `ddd ddd dddd` /
/// `(ddd) ddd-dddd` shapes, with an optional `+1` country code. Separators between the groups are
/// **required** — a bare 10-digit run is too ambiguous to flag.
fn scan_us_phone(bytes: &[u8], out: &mut Vec<agent_abi::Finding>) {
    let mut i = 0;
    while i < bytes.len() {
        let boundary_before = i == 0 || !bytes[i - 1].is_ascii_digit();
        let could_start = bytes[i] == b'+' || bytes[i] == b'(' || bytes[i].is_ascii_digit();
        if boundary_before && could_start {
            if let Some(end) = parse_us_phone(bytes, i) {
                if end == bytes.len() || !bytes[end].is_ascii_digit() {
                    push_finding(out, i, end, "pii.us_phone", 0.8);
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
}

fn parse_us_phone(bytes: &[u8], start: usize) -> Option<usize> {
    let mut j = start;
    // Optional +1 country code, then separators.
    if bytes.get(j) == Some(&b'+') {
        if bytes.get(j + 1) != Some(&b'1') {
            return None;
        }
        j = skip_seps(bytes, j + 2);
    }
    // Area: optional parens.
    let paren = bytes.get(j) == Some(&b'(');
    if paren {
        j += 1;
    }
    j = read_digits(bytes, j, 3)?;
    if paren {
        if bytes.get(j) != Some(&b')') {
            return None;
        }
        j += 1;
    }
    // Required separator, prefix group, required separator, line group.
    let after = skip_seps(bytes, j);
    if after == j {
        return None;
    }
    j = read_digits(bytes, after, 3)?;
    let after = skip_seps(bytes, j);
    if after == j {
        return None;
    }
    read_digits(bytes, after, 4)
}

// --- US SSN ----------------------------------------------------------------------------------

/// Detect a US SSN in `ddd-dd-dddd` form (dashes required), rejecting structurally invalid areas
/// (000, 666, 900–999), groups (00), and serials (0000).
fn scan_us_ssn(bytes: &[u8], out: &mut Vec<agent_abi::Finding>) {
    const LEN: usize = 11;
    let mut i = 0;
    while i + LEN <= bytes.len() {
        let end = i + LEN;
        let boundary_before = i == 0 || !bytes[i - 1].is_ascii_digit();
        let boundary_after =
            end == bytes.len() || !(bytes[end].is_ascii_digit() || bytes[end] == b'-');
        if boundary_before && boundary_after && is_ssn(&bytes[i..end]) {
            push_finding(out, i, end, "pii.us_ssn", 0.85);
            i = end;
            continue;
        }
        i += 1;
    }
}

fn is_ssn(w: &[u8]) -> bool {
    if w[3] != b'-' || w[6] != b'-' {
        return false;
    }
    if ![0, 1, 2, 4, 5, 7, 8, 9, 10]
        .iter()
        .all(|&k| w[k].is_ascii_digit())
    {
        return false;
    }
    let area = digits_to_num(&w[0..3]);
    let group = digits_to_num(&w[4..6]);
    let serial = digits_to_num(&w[7..11]);
    area != 0 && area != 666 && area < 900 && group != 0 && serial != 0
}

fn digits_to_num(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |a, &b| a * 10 + u32::from(b - b'0'))
}

// --- credit card -----------------------------------------------------------------------------

/// Detect a 13–19 digit payment-card number (single space/dash separators allowed) that passes the
/// Luhn checksum — the checksum is what holds precision against arbitrary long digit runs.
fn scan_credit_card(bytes: &[u8], out: &mut Vec<agent_abi::Finding>) {
    let mut i = 0;
    while i < bytes.len() {
        let boundary_before = i == 0 || !bytes[i - 1].is_ascii_digit();
        if boundary_before && bytes[i].is_ascii_digit() {
            let (end, digits) = read_card_run(bytes, i);
            if (13..=19).contains(&digits.len())
                && luhn_ok(&digits)
                && (end == bytes.len() || !bytes[end].is_ascii_digit())
            {
                push_finding(out, i, end, "pii.credit_card", 0.9);
                i = end;
                continue;
            }
        }
        i += 1;
    }
}

/// Read a run of digits with optional single space/dash separators between them; returns the index
/// just past the last digit and the collected digit values.
fn read_card_run(bytes: &[u8], start: usize) -> (usize, Vec<u8>) {
    let mut digits = Vec::new();
    let mut j = start;
    let mut last_digit_end = start;
    while j < bytes.len() {
        let b = bytes[j];
        if b.is_ascii_digit() {
            digits.push(b - b'0');
            j += 1;
            last_digit_end = j;
        } else if matches!(b, b' ' | b'-') && bytes.get(j + 1).is_some_and(|n| n.is_ascii_digit()) {
            j += 1;
        } else {
            break;
        }
    }
    (last_digit_end, digits)
}

fn luhn_ok(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    let mut double = false;
    for &d in digits.iter().rev() {
        let mut v = u32::from(d);
        if double {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
        double = !double;
    }
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(v: &Verdict) -> Vec<&str> {
        v.findings.iter().map(|f| f.label.as_str()).collect()
    }

    #[test]
    fn flags_an_email_with_its_span() {
        let v = detect("reach me at jane.doe@example.com please");
        let email: Vec<_> = v
            .findings
            .iter()
            .filter(|f| f.label == "pii.email")
            .collect();
        assert_eq!(email.len(), 1);
        assert_eq!(
            &"reach me at jane.doe@example.com please"
                [email[0].span.start as usize..email[0].span.end as usize],
            "jane.doe@example.com"
        );
    }

    #[test]
    fn flags_a_valid_ipv4_but_not_an_out_of_range_one() {
        assert!(labels(&detect("host 192.168.1.1 up")).contains(&"pii.ipv4"));
        assert!(!labels(&detect("host 999.1.1.1 up")).contains(&"pii.ipv4"));
    }

    #[test]
    fn flags_a_us_phone_with_separators_only() {
        assert!(labels(&detect("call 415-555-0132")).contains(&"pii.us_phone"));
        assert!(labels(&detect("call (415) 555-0132")).contains(&"pii.us_phone"));
        // A bare 10-digit run is too ambiguous — not flagged as a phone.
        assert!(!labels(&detect("id 4155550132 end")).contains(&"pii.us_phone"));
    }

    #[test]
    fn flags_a_structurally_valid_ssn_but_not_an_invalid_area() {
        assert!(labels(&detect("ssn 123-45-6789")).contains(&"pii.us_ssn"));
        assert!(!labels(&detect("ssn 666-45-6789")).contains(&"pii.us_ssn"));
        assert!(!labels(&detect("ssn 000-45-6789")).contains(&"pii.us_ssn"));
    }

    #[test]
    fn flags_a_luhn_valid_card_but_not_an_invalid_one() {
        // 4111 1111 1111 1111 is the public Visa test number (Luhn-valid, not a real card).
        assert!(labels(&detect("card 4111 1111 1111 1111 ok")).contains(&"pii.credit_card"));
        assert!(!labels(&detect("card 4111 1111 1111 1112 ok")).contains(&"pii.credit_card"));
    }

    #[test]
    fn clean_prose_has_no_findings() {
        assert!(!detect("the quick brown fox jumps over the lazy dog").fired());
    }

    #[test]
    fn is_deterministic() {
        let text = "jane.doe@example.com from 192.168.1.1 at 415-555-0132";
        assert_eq!(detect(text), detect(text));
    }
}
