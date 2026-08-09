//! Tests for synced workflow preferences and their backward-compatible defaults.

use claria_desktop::config::{
    ClariaConfig, CredentialSource, ReportAuthoringPreferences, SyncedPreferences,
    TranscriptionLanguage, TranscriptionPreferences,
};

fn sample_config() -> ClariaConfig {
    ClariaConfig {
        config_version: 6,
        region: "us-east-1".into(),
        system_name: "test".into(),
        account_id: "123456789012".into(),
        created_at: jiff::Timestamp::UNIX_EPOCH,
        credentials: CredentialSource::DefaultChain,
        preferred_model_id: Some("anthropic.claude-opus-4-7".into()),
        cost_explorer_enabled: true,
        hourly_cost_data: false,
        prompt_caching_enabled: true,
        transcription: TranscriptionPreferences {
            default_language: TranscriptionLanguage::Mixed,
            default_speaker_count: 3,
            use_medical_for_english: true,
            translate_to_english: true,
        },
        report_authoring: ReportAuthoringPreferences::default(),
    }
}

#[test]
fn synced_preferences_round_trip() {
    let cfg = sample_config();
    let synced = SyncedPreferences::from_config(&cfg);

    assert_eq!(
        synced.preferred_model_id,
        Some("anthropic.claude-opus-4-7".into())
    );
    assert!(synced.cost_explorer_enabled);
    assert_eq!(
        synced.transcription.default_language,
        TranscriptionLanguage::Mixed
    );
    assert_eq!(synced.transcription.default_speaker_count, 3);
    assert!(synced.transcription.use_medical_for_english);
}

#[test]
fn apply_to_config_leaves_machine_local_fields_alone() {
    let mut cfg = ClariaConfig {
        region: "us-west-2".into(),
        system_name: "machine-b".into(),
        account_id: "999999999999".into(),
        ..sample_config()
    };

    let synced = SyncedPreferences {
        preferences_version: 1,
        preferred_model_id: Some("anthropic.claude-sonnet-4-6".into()),
        cost_explorer_enabled: false,
        hourly_cost_data: true,
        prompt_caching_enabled: false,
        transcription: TranscriptionPreferences::default(),
        report_authoring: ReportAuthoringPreferences {
            max_tool_rounds: 12,
            max_converse_calls: 13,
            max_tool_uses_per_response: 16,
            max_retained_turns: 30,
        },
    };

    synced.apply_to_config(&mut cfg);

    // Machine-local untouched.
    assert_eq!(cfg.region, "us-west-2");
    assert_eq!(cfg.system_name, "machine-b");
    assert_eq!(cfg.account_id, "999999999999");

    // Synced fields applied.
    assert_eq!(
        cfg.preferred_model_id,
        Some("anthropic.claude-sonnet-4-6".into())
    );
    assert!(!cfg.cost_explorer_enabled);
    assert!(cfg.hourly_cost_data);
    assert!(!cfg.prompt_caching_enabled);
    assert_eq!(
        cfg.transcription.default_language,
        TranscriptionLanguage::English
    );
    assert_eq!(cfg.report_authoring.max_tool_rounds, 12);
    assert_eq!(cfg.report_authoring.max_converse_calls, 13);
}

#[test]
fn synced_preferences_serialize_snake_case() {
    let synced = SyncedPreferences::from_config(&sample_config());
    let json = serde_json::to_string(&synced).unwrap();

    assert!(json.contains("\"default_language\":\"mixed\""));
    assert!(json.contains("\"use_medical_for_english\":true"));
    assert!(json.contains("\"max_tool_rounds\":40"));
    assert!(json.contains("\"preferences_version\":2"));
}

#[test]
fn legacy_v5_config_migrates_to_v6_with_default_transcription() {
    // A pre-transcription config JSON as stored on disk under config v5.
    let raw = serde_json::json!({
        "config_version": 5,
        "region": "us-east-1",
        "system_name": "test",
        "account_id": "123456789012",
        "created_at": "1970-01-01T00:00:00Z",
        "credentials": { "type": "default_chain" },
        "preferred_model_id": null,
        "cost_explorer_enabled": false,
        "hourly_cost_data": false,
        "prompt_caching_enabled": true,
    });

    let cfg: ClariaConfig = serde_json::from_value(raw).unwrap();

    // `#[serde(default)]` fills in transcription preferences.
    assert_eq!(
        cfg.transcription.default_language,
        TranscriptionLanguage::English
    );
    assert_eq!(cfg.transcription.default_speaker_count, 2);
    assert!(!cfg.transcription.use_medical_for_english);
    assert_eq!(cfg.report_authoring.max_tool_rounds, 40);
    assert_eq!(cfg.report_authoring.max_converse_calls, 50);
    assert_eq!(cfg.report_authoring.max_tool_uses_per_response, 80);
    assert_eq!(cfg.report_authoring.max_retained_turns, 200);
}

#[test]
fn invalid_report_authoring_preferences_are_rejected() {
    let invalid = ReportAuthoringPreferences {
        max_converse_calls: 0,
        ..ReportAuthoringPreferences::default()
    };

    assert!(invalid.validate().is_err());
}
