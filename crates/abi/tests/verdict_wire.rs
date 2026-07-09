//! P1.3: the `Verdict` wire contract is sacred. These tests prove the framed ABI buffer
//! round-trips and **pin the exact JSON shape** so a field can never be renamed, reordered, or
//! retyped without a deliberate, visible change here.

use agent_abi::{Finding, Provenance, Span, Verdict};

fn sample() -> Verdict {
    Verdict::new(
        0,
        vec![Finding::new("keyword.badword", 1.0, Span::new(10, 17))],
        Provenance::new("mock", "0.1.0", 0.5),
    )
}

#[test]
fn verdict_framing_round_trips() {
    let v = sample();
    let bytes = v.encode().unwrap();
    assert_eq!(Verdict::decode(&bytes).unwrap(), v);
}

#[test]
fn clean_verdict_has_not_fired() {
    let v = Verdict::clean(Provenance::new("mock", "0.1.0", 0.5));
    assert!(!v.fired());
    assert!(sample().fired());
}

#[test]
fn verdict_json_shape_is_pinned() {
    let expected = r#"{
  "abi_version": 0,
  "findings": [
    {
      "label": "keyword.badword",
      "score": 1.0,
      "span": {
        "start": 10,
        "end": 17
      }
    }
  ],
  "provenance": {
    "detector_id": "mock",
    "detector_version": "0.1.0",
    "threshold": 0.5,
    "scorecard_hash": null
  }
}"#;
    assert_eq!(serde_json::to_string_pretty(&sample()).unwrap(), expected);
}
