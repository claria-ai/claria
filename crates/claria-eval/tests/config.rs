//! Reading the desktop's `config.json` without the desktop.

use claria_eval::config::{self, CredentialSource};

/// A config carrying fields this tool has never heard of — every preference
/// block the desktop stores, plus a `config_version` from a future build.
const CONFIG_WITH_UNKNOWN_FIELDS: &str = r#"{
  "config_version": 999,
  "region": "us-east-1",
  "system_name": "smoke",
  "account_id": "123456789012",
  "created_at": "2026-01-01T00:00:00Z",
  "credentials": {
    "type": "inline",
    "access_key_id": "AKIAEXAMPLE",
    "secret_access_key": "wJalrEXAMPLEKEY"
  },
  "preferred_model_id": "us.anthropic.claude-opus-4-6-20260101-v1:0",
  "cost_explorer_enabled": true,
  "hourly_cost_data": false,
  "prompt_caching_enabled": true,
  "transcription": {"default_language": "english", "default_speaker_count": 2},
  "report_authoring": {"max_tool_rounds": 40},
  "model_tuning": {},
  "chat_streaming": "incremental",
  "draft_pipeline": {"plan_gate": "gated", "planner_model_id": null},
  "a_field_from_a_later_config_version": {"nested": [1, 2, 3]}
}"#;

#[test]
fn only_the_fields_the_harness_needs_are_read() {
    let config = config::parse(CONFIG_WITH_UNKNOWN_FIELDS.as_bytes()).expect("parse");
    assert_eq!(config.region, "us-east-1");
    assert_eq!(config.system_name, "smoke");
    assert_eq!(config.account_id, "123456789012");
    assert_eq!(
        config.preferred_model_id.as_deref(),
        Some("us.anthropic.claude-opus-4-6-20260101-v1:0")
    );
    assert_eq!(
        config.credentials,
        CredentialSource::Inline {
            access_key_id: "AKIAEXAMPLE".to_string(),
            secret_access_key: "wJalrEXAMPLEKEY".to_string(),
            session_token: None,
        }
    );
}

#[test]
fn the_bucket_is_derived_the_way_the_provisioner_named_it() {
    let config = config::parse(CONFIG_WITH_UNKNOWN_FIELDS.as_bytes()).expect("parse");
    assert_eq!(config.bucket().expect("bucket"), "123456789012-smoke-data");
}

/// A security-scoping value fails closed: no account ID means no bucket name,
/// not a bucket name with a hole in it.
#[test]
fn a_config_without_an_account_id_has_no_bucket() {
    let config = config::parse(
        br#"{"region":"us-east-1","system_name":"smoke","credentials":{"type":"default_chain"}}"#,
    )
    .expect("parse");
    assert_eq!(config.account_id, "");
    let error = config.bucket().expect_err("no bucket without an account");
    assert!(format!("{error}").contains("account ID"));
}

#[test]
fn every_credential_source_the_desktop_writes_round_trips() {
    let profile = config::parse(
        br#"{"region":"eu-west-1","system_name":"s","account_id":"1","credentials":
             {"type":"profile","profile_name":"claria"}}"#,
    )
    .expect("parse profile");
    assert_eq!(
        profile.credentials,
        CredentialSource::Profile {
            profile_name: "claria".to_string()
        }
    );

    let chain = config::parse(
        br#"{"region":"eu-west-1","system_name":"s","account_id":"1","credentials":
             {"type":"default_chain"}}"#,
    )
    .expect("parse default chain");
    assert_eq!(chain.credentials, CredentialSource::DefaultChain);

    let session = config::parse(
        br#"{"region":"eu-west-1","system_name":"s","account_id":"1","credentials":
             {"type":"inline","access_key_id":"a","secret_access_key":"b","session_token":"c"}}"#,
    )
    .expect("parse session credentials");
    assert_eq!(
        session.credentials,
        CredentialSource::Inline {
            access_key_id: "a".to_string(),
            secret_access_key: "b".to_string(),
            session_token: Some("c".to_string()),
        }
    );
}

#[test]
fn a_config_missing_a_required_field_says_so() {
    let error = config::parse(br#"{"system_name":"smoke"}"#).expect_err("no region, no config");
    assert!(format!("{error}").contains("did not parse"));
}

// ---------------------------------------------------------------------------
// The headless path: no desktop, no config.json
// ---------------------------------------------------------------------------

/// The three values this tool cannot discover for itself, supplied directly.
#[test]
fn explicit_parts_build_a_config_without_a_file() {
    let config = config::from_parts(
        Some("us-west-2".to_string()),
        Some("123456789012".to_string()),
        Some("smoke".to_string()),
    )
    .expect("explicit parts are enough");

    assert_eq!(config.region, "us-west-2");
    assert_eq!(config.credentials, CredentialSource::DefaultChain);
    assert_eq!(config.bucket().expect("bucket"), {
        claria_core::s3_keys::bucket_name("123456789012", "smoke")
    });
}

/// A security-scoping value fails closed. An empty account ID must not resolve
/// to some other account's bucket.
#[test]
fn a_blank_account_id_is_refused() {
    let error = config::from_parts(
        Some("us-east-1".to_string()),
        Some("   ".to_string()),
        Some("smoke".to_string()),
    )
    .expect_err("a blank account ID cannot name a bucket");
    assert!(format!("{error}").contains("account ID"), "{error}");
}

#[test]
fn a_missing_region_names_the_flag_and_the_variable() {
    let error = config::from_parts(None, Some("123456789012".to_string()), None)
        .expect_err("no region");
    let message = format!("{error}");
    assert!(message.contains("--region"), "{message}");
    assert!(message.contains(config::REGION_ENV), "{message}");
}

/// Naming nothing at all means the desktop config is what was intended.
#[test]
fn nothing_supplied_is_not_a_headless_request() {
    // Only meaningful when the environment is clean; the variables are read
    // per-process and this test does not set them.
    if std::env::var_os(config::REGION_ENV).is_some()
        || std::env::var_os(config::ACCOUNT_ID_ENV).is_some()
        || std::env::var_os(config::SYSTEM_NAME_ENV).is_some()
    {
        return;
    }
    assert!(!config::headless_requested(None, None, None));
}

/// One flag is enough to mean it: a half-supplied set should say which part is
/// missing rather than quietly read a file that happens to exist.
#[test]
fn one_flag_is_a_headless_request() {
    assert!(config::headless_requested(Some("us-east-1"), None, None));
}
