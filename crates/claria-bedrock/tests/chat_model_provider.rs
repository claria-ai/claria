use claria_bedrock::geography::{AvailableProfile, FoundationModelStatus};
use claria_bedrock::live_aws::{build_candidates, classify_current, CurrentValidity};
use jiff::Timestamp;
use serde_json::{json, Value};

fn active(id: &str) -> AvailableProfile {
    AvailableProfile {
        profile_id: id.into(),
        underlying_status: FoundationModelStatus::Active,
    }
}

fn legacy(id: &str, eol: Timestamp) -> AvailableProfile {
    AvailableProfile {
        profile_id: id.into(),
        underlying_status: FoundationModelStatus::Legacy { eol },
    }
}

fn eol_p(id: &str) -> AvailableProfile {
    AvailableProfile {
        profile_id: id.into(),
        underlying_status: FoundationModelStatus::Eol,
    }
}

#[test]
fn classify_active_profile_returns_active() {
    let profiles = vec![active("us.anthropic.claude-sonnet-4-20250514-v1:0")];
    let current = json!("us.anthropic.claude-sonnet-4-20250514-v1:0");
    assert!(matches!(
        classify_current(&current, &profiles, "us"),
        CurrentValidity::Active
    ));
}

#[test]
fn classify_unknown_profile_returns_retired() {
    let profiles = vec![active("us.anthropic.claude-opus-4-20250514-v1:0")];
    let current = json!("us.anthropic.claude-sonnet-3-20240101-v1:0");
    assert!(matches!(
        classify_current(&current, &profiles, "us"),
        CurrentValidity::Retired
    ));
}

#[test]
fn classify_legacy_profile_returns_deprecation() {
    let eol: Timestamp = "2026-12-01T00:00:00Z".parse().unwrap();
    let profiles = vec![legacy(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        eol,
    )];
    let current = json!("us.anthropic.claude-sonnet-4-20250514-v1:0");
    match classify_current(&current, &profiles, "us") {
        CurrentValidity::Deprecation { eol: s } => {
            assert!(s.contains("2026-12-01"), "eol string: {s}");
        }
        other => panic!("expected Deprecation, got {other:?}"),
    }
}

#[test]
fn classify_eol_profile_returns_retired() {
    let profiles = vec![eol_p("us.anthropic.claude-sonnet-3-20240101-v1:0")];
    let current = json!("us.anthropic.claude-sonnet-3-20240101-v1:0");
    assert!(matches!(
        classify_current(&current, &profiles, "us"),
        CurrentValidity::Retired
    ));
}

#[test]
fn classify_bare_model_id_returns_malformed() {
    let profiles = vec![active("us.anthropic.claude-sonnet-4-20250514-v1:0")];
    let current = json!("anthropic.claude-3-haiku-20240307-v1:0");
    assert!(matches!(
        classify_current(&current, &profiles, "us"),
        CurrentValidity::Malformed
    ));
}

#[test]
fn classify_application_arn_returns_malformed() {
    let profiles = vec![];
    let current = json!("arn:aws:bedrock:us-east-1:123:application-inference-profile/abc");
    assert!(matches!(
        classify_current(&current, &profiles, "us"),
        CurrentValidity::Malformed
    ));
}

#[test]
fn classify_null_returns_never_configured() {
    let profiles = vec![active("us.anthropic.claude-sonnet-4-20250514-v1:0")];
    assert!(matches!(
        classify_current(&Value::Null, &profiles, "us"),
        CurrentValidity::NeverConfigured
    ));
}

#[test]
fn classify_empty_string_returns_never_configured() {
    let profiles = vec![active("us.anthropic.claude-sonnet-4-20250514-v1:0")];
    let current = json!("");
    assert!(matches!(
        classify_current(&current, &profiles, "us"),
        CurrentValidity::NeverConfigured
    ));
}

#[test]
fn classify_eu_profile_when_user_prefers_us_returns_preference_mismatch() {
    let profiles = vec![active("us.anthropic.claude-sonnet-4-20250514-v1:0")];
    let current = json!("eu.anthropic.claude-sonnet-4-20250514-v1:0");
    assert!(matches!(
        classify_current(&current, &profiles, "us"),
        CurrentValidity::PreferenceMismatch
    ));
}

#[test]
fn build_candidates_filters_to_active_only() {
    let eol: Timestamp = "2026-12-01T00:00:00Z".parse().unwrap();
    let profiles = vec![
        active("us.anthropic.claude-sonnet-4-20250514-v1:0"),
        legacy("us.anthropic.claude-sonnet-3-20240601-v1:0", eol),
        eol_p("us.anthropic.claude-sonnet-2-20230101-v1:0"),
    ];
    let candidates = build_candidates(&profiles, false);
    assert_eq!(candidates.len(), 1, "only ACTIVE should be a candidate");
    assert_eq!(
        candidates[0].value,
        json!("us.anthropic.claude-sonnet-4-20250514-v1:0")
    );
}

#[test]
fn build_candidates_returns_empty_for_no_active() {
    let profiles = vec![eol_p("us.anthropic.claude-sonnet-3-20240101-v1:0")];
    let candidates = build_candidates(&profiles, false);
    assert!(candidates.is_empty());
}

#[test]
fn build_candidates_prefers_sonnet_over_opus_for_recommendation() {
    let profiles = vec![
        active("us.anthropic.claude-opus-4-20250514-v1:0"),
        active("us.anthropic.claude-sonnet-4-20250514-v1:0"),
    ];
    let candidates = build_candidates(&profiles, false);
    // Both included
    assert_eq!(candidates.len(), 2);
    // Only Sonnet recommended
    let recommended: Vec<&serde_json::Value> = candidates
        .iter()
        .filter(|c| c.recommended)
        .map(|c| &c.value)
        .collect();
    assert_eq!(recommended.len(), 1);
    assert_eq!(
        recommended[0],
        &json!("us.anthropic.claude-sonnet-4-20250514-v1:0")
    );
}

#[test]
fn build_candidates_picks_newest_sonnet_when_chat() {
    // chat = prefer_cheapest=false → pick newest (highest lexicographic)
    let profiles = vec![
        active("us.anthropic.claude-sonnet-4-20250514-v1:0"),
        active("us.anthropic.claude-sonnet-4-6-20260301-v1:0"),
    ];
    let candidates = build_candidates(&profiles, false);
    let recommended: Vec<&serde_json::Value> = candidates
        .iter()
        .filter(|c| c.recommended)
        .map(|c| &c.value)
        .collect();
    assert_eq!(recommended.len(), 1);
    assert_eq!(
        recommended[0],
        &json!("us.anthropic.claude-sonnet-4-6-20260301-v1:0")
    );
}

#[test]
fn build_candidates_picks_cheapest_sonnet_when_extraction() {
    // extraction = prefer_cheapest=true → pick lowest (oldest)
    let profiles = vec![
        active("us.anthropic.claude-sonnet-4-20250514-v1:0"),
        active("us.anthropic.claude-sonnet-4-6-20260301-v1:0"),
    ];
    let candidates = build_candidates(&profiles, true);
    let recommended: Vec<&serde_json::Value> = candidates
        .iter()
        .filter(|c| c.recommended)
        .map(|c| &c.value)
        .collect();
    assert_eq!(recommended.len(), 1);
    assert_eq!(
        recommended[0],
        &json!("us.anthropic.claude-sonnet-4-20250514-v1:0")
    );
}

#[test]
fn build_candidates_falls_back_to_opus_when_no_sonnet() {
    let profiles = vec![
        active("us.anthropic.claude-opus-4-20250514-v1:0"),
        active("us.anthropic.claude-opus-4-5-20251101-v1:0"),
    ];
    let candidates = build_candidates(&profiles, false);
    let recommended: Vec<&serde_json::Value> = candidates
        .iter()
        .filter(|c| c.recommended)
        .map(|c| &c.value)
        .collect();
    assert_eq!(recommended.len(), 1);
    // Newest opus
    assert_eq!(
        recommended[0],
        &json!("us.anthropic.claude-opus-4-5-20251101-v1:0")
    );
}

#[test]
fn build_candidates_sorts_results_by_profile_id() {
    let profiles = vec![
        active("us.anthropic.claude-sonnet-4-6-20260301-v1:0"),
        active("us.anthropic.claude-opus-4-20250514-v1:0"),
        active("us.anthropic.claude-sonnet-4-20250514-v1:0"),
    ];
    let candidates = build_candidates(&profiles, false);
    assert_eq!(candidates.len(), 3);
    // Should be sorted by profile_id (lexicographic)
    for window in candidates.windows(2) {
        let a = window[0].value.as_str().unwrap();
        let b = window[1].value.as_str().unwrap();
        assert!(a <= b, "candidates not sorted: {a} vs {b}");
    }
}
