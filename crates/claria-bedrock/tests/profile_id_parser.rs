use claria_bedrock::geography::parse_inference_profile_id;

#[test]
fn parses_us_anthropic_profile() {
    let id = "us.anthropic.claude-sonnet-4-20250514-v1:0";
    assert_eq!(
        parse_inference_profile_id(id),
        Some(("us", "anthropic.claude-sonnet-4-20250514-v1:0"))
    );
}

#[test]
fn parses_eu_anthropic_profile() {
    let id = "eu.anthropic.claude-sonnet-4-20250514-v1:0";
    assert_eq!(
        parse_inference_profile_id(id),
        Some(("eu", "anthropic.claude-sonnet-4-20250514-v1:0"))
    );
}

#[test]
fn parses_apac_anthropic_profile() {
    let id = "apac.anthropic.claude-sonnet-4-20250514-v1:0";
    assert_eq!(
        parse_inference_profile_id(id),
        Some(("apac", "anthropic.claude-sonnet-4-20250514-v1:0"))
    );
}

#[test]
fn parses_multipart_ap_jp_prefix() {
    let id = "ap-jp.anthropic.claude-sonnet-4-20250514-v1:0";
    assert_eq!(
        parse_inference_profile_id(id),
        Some(("ap-jp", "anthropic.claude-sonnet-4-20250514-v1:0"))
    );
}

#[test]
fn parses_multipart_ap_au_prefix() {
    let id = "ap-au.anthropic.claude-sonnet-4-20250514-v1:0";
    assert_eq!(
        parse_inference_profile_id(id),
        Some(("ap-au", "anthropic.claude-sonnet-4-20250514-v1:0"))
    );
}

#[test]
fn parses_global_profile() {
    let id = "global.anthropic.claude-sonnet-4-20250514-v1:0";
    assert_eq!(
        parse_inference_profile_id(id),
        Some(("global", "anthropic.claude-sonnet-4-20250514-v1:0"))
    );
}

#[test]
fn rejects_bare_model_id() {
    let id = "anthropic.claude-3-5-sonnet-20241022-v2:0";
    assert_eq!(parse_inference_profile_id(id), None);
}

#[test]
fn rejects_application_profile_arn() {
    let id = "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/abc123";
    assert_eq!(parse_inference_profile_id(id), None);
}

#[test]
fn rejects_empty_string() {
    assert_eq!(parse_inference_profile_id(""), None);
}

#[test]
fn rejects_single_segment() {
    assert_eq!(parse_inference_profile_id("anthropic"), None);
}

#[test]
fn rejects_two_segments_only() {
    // Two segments looks like a bare model ID, not a profile ID
    assert_eq!(parse_inference_profile_id("us.anthropic"), None);
}

#[test]
fn rejects_uppercase_scope() {
    // Scope prefixes are always lowercase in AWS conventions
    assert_eq!(
        parse_inference_profile_id("US.anthropic.claude-sonnet-4-20250514-v1:0"),
        None
    );
}

#[test]
fn rejects_empty_scope() {
    assert_eq!(
        parse_inference_profile_id(".anthropic.claude-sonnet-4-20250514-v1:0"),
        None
    );
}

#[test]
fn rejects_empty_provider() {
    assert_eq!(
        parse_inference_profile_id("us..claude-sonnet-4-20250514-v1:0"),
        None
    );
}

#[test]
fn rejects_empty_model() {
    assert_eq!(parse_inference_profile_id("us.anthropic."), None);
}

#[test]
fn rejects_double_hyphen_in_scope() {
    assert_eq!(
        parse_inference_profile_id("ap--jp.anthropic.claude-sonnet-4-20250514-v1:0"),
        None
    );
}

#[test]
fn round_trips_via_format_macro() {
    let original = "us.anthropic.claude-sonnet-4-20250514-v1:0";
    let (geo, bare) = parse_inference_profile_id(original).expect("parses");
    let reassembled = format!("{geo}.{bare}");
    assert_eq!(reassembled, original);
}

#[test]
fn round_trips_for_multipart_geography() {
    let original = "ap-jp.anthropic.claude-sonnet-4-20250514-v1:0";
    let (geo, bare) = parse_inference_profile_id(original).expect("parses");
    let reassembled = format!("{geo}.{bare}");
    assert_eq!(reassembled, original);
}
