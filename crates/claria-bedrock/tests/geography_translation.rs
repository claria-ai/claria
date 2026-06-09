use claria_bedrock::geography::{
    AvailableProfile, FoundationModelStatus, GeographyTranslation, RepickReason,
    translate_profile_for_geography,
};
use jiff::Timestamp;

fn active(profile_id: &str) -> AvailableProfile {
    AvailableProfile {
        profile_id: profile_id.into(),
        underlying_status: FoundationModelStatus::Active,
    }
}

fn legacy(profile_id: &str, eol: Timestamp) -> AvailableProfile {
    AvailableProfile {
        profile_id: profile_id.into(),
        underlying_status: FoundationModelStatus::Legacy { eol },
    }
}

fn eol_profile(profile_id: &str) -> AvailableProfile {
    AvailableProfile {
        profile_id: profile_id.into(),
        underlying_status: FoundationModelStatus::Eol,
    }
}

#[test]
fn swap_us_to_eu_when_model_exists() {
    let result = translate_profile_for_geography(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "eu",
        &[active("eu.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::SilentSwap {
            new_value: "eu.anthropic.claude-sonnet-4-20250514-v1:0".into()
        }
    );
}

#[test]
fn requires_repick_when_constructed_id_absent() {
    let result = translate_profile_for_geography(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "eu",
        &[active("eu.anthropic.claude-opus-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::RequiresRepick {
            reason: RepickReason::ProfileNotAvailableInGeography
        }
    );
}

#[test]
fn no_change_when_already_in_target_geography() {
    let result = translate_profile_for_geography(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "us",
        &[active("us.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(result, GeographyTranslation::NoChangeNeeded);
}

#[test]
fn requires_repick_when_current_is_bare_id() {
    let result = translate_profile_for_geography(
        "anthropic.claude-3-haiku-20240307-v1:0",
        "us",
        &[active("us.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::RequiresRepick {
            reason: RepickReason::CurrentValueIsBareModelId
        }
    );
}

#[test]
fn requires_repick_when_current_is_application_profile_arn() {
    let result = translate_profile_for_geography(
        "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/abc123",
        "us",
        &[active("us.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::RequiresRepick {
            reason: RepickReason::CurrentValueIsCustomApplicationProfile
        }
    );
}

#[test]
fn requires_repick_when_current_is_empty() {
    let result = translate_profile_for_geography(
        "",
        "us",
        &[active("us.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::RequiresRepick {
            reason: RepickReason::CurrentValueMalformed
        }
    );
}

#[test]
fn requires_repick_when_current_is_garbage() {
    let result = translate_profile_for_geography(
        "not-a-real-id",
        "us",
        &[active("us.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::RequiresRepick {
            reason: RepickReason::CurrentValueMalformed
        }
    );
}

#[test]
fn swap_us_to_global_when_global_profile_exists() {
    let result = translate_profile_for_geography(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "global",
        &[active("global.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::SilentSwap {
            new_value: "global.anthropic.claude-sonnet-4-20250514-v1:0".into()
        }
    );
}

#[test]
fn requires_repick_when_global_profile_missing() {
    // Common case: not every model has a global. variant
    let result = translate_profile_for_geography(
        "us.anthropic.claude-opus-4-20250514-v1:0",
        "global",
        &[active("global.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::RequiresRepick {
            reason: RepickReason::ProfileNotAvailableInGeography
        }
    );
}

#[test]
fn swap_apac_to_ap_jp_when_model_supported() {
    let result = translate_profile_for_geography(
        "apac.anthropic.claude-sonnet-4-20250514-v1:0",
        "ap-jp",
        &[active("ap-jp.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::SilentSwap {
            new_value: "ap-jp.anthropic.claude-sonnet-4-20250514-v1:0".into()
        }
    );
}

#[test]
fn requires_repick_when_swap_target_is_legacy() {
    let eol: Timestamp = "2026-12-01T00:00:00Z".parse().unwrap();
    let result = translate_profile_for_geography(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "eu",
        &[legacy("eu.anthropic.claude-sonnet-4-20250514-v1:0", eol)],
    );
    assert_eq!(
        result,
        GeographyTranslation::RequiresRepick {
            reason: RepickReason::UnderlyingModelLegacy { eol }
        }
    );
}

#[test]
fn requires_repick_when_swap_target_is_eol() {
    let result = translate_profile_for_geography(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "eu",
        &[eol_profile("eu.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::RequiresRepick {
            reason: RepickReason::UnderlyingModelEol
        }
    );
}

#[test]
fn swap_preserves_full_version_suffix() {
    // The bare model ID — version suffix and all — must pass through verbatim
    let result = translate_profile_for_geography(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "eu",
        &[active("eu.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    match result {
        GeographyTranslation::SilentSwap { new_value } => {
            assert_eq!(new_value, "eu.anthropic.claude-sonnet-4-20250514-v1:0");
        }
        other => panic!("expected SilentSwap, got {other:?}"),
    }
}

#[test]
fn swap_handles_multipart_geography_prefix() {
    // ap-jp is parsed as one prefix, not split on the hyphen
    let result = translate_profile_for_geography(
        "ap-jp.anthropic.claude-sonnet-4-20250514-v1:0",
        "us",
        &[active("us.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::SilentSwap {
            new_value: "us.anthropic.claude-sonnet-4-20250514-v1:0".into()
        }
    );
}

#[test]
fn requires_repick_when_geography_value_is_unknown() {
    // Defensive: UI shouldn't surface this, but function shouldn't panic
    let result = translate_profile_for_geography(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "mars",
        &[active("us.anthropic.claude-sonnet-4-20250514-v1:0")],
    );
    assert_eq!(
        result,
        GeographyTranslation::RequiresRepick {
            reason: RepickReason::CurrentValueMalformed
        }
    );
}
