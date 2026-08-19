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
