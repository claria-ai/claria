//! Tests for synced workflow preferences and their backward-compatible defaults.

use claria_desktop::config::{
    ChatStreamMode, ClariaConfig, CredentialSource, DraftPipelinePreferences, PlanGateMode,
    ReportAuthoringPreferences, SecuritySettings, SyncedPreferences, TranscriptionLanguage,
    TranscriptionPreferences,
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
        model_tuning: Default::default(),
        chat_streaming: ChatStreamMode::Token,
        draft_pipeline: DraftPipelinePreferences::default(),
        security: SecuritySettings::default(),
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
            ..ReportAuthoringPreferences::default()
        },
        model_tuning: Default::default(),
        chat_streaming: ChatStreamMode::Off,
        draft_pipeline: DraftPipelinePreferences {
            plan_gate: PlanGateMode::AutoStart,
            planner_model_id: Some("us.anthropic.claude-haiku-4-5-20251001-v1:0".into()),
            reviewer_model_id: None,
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
    assert_eq!(cfg.chat_streaming, ChatStreamMode::Off);
    assert_eq!(cfg.draft_pipeline.plan_gate, PlanGateMode::AutoStart);
    assert_eq!(
        cfg.draft_pipeline.planner_model_id.as_deref(),
        Some("us.anthropic.claude-haiku-4-5-20251001-v1:0")
    );
}

#[test]
fn synced_preferences_serialize_snake_case() {
    let synced = SyncedPreferences::from_config(&sample_config());
    let json = serde_json::to_string(&synced).unwrap();

    assert!(json.contains("\"default_language\":\"mixed\""));
    assert!(json.contains("\"use_medical_for_english\":true"));
    assert!(json.contains("\"max_tool_rounds\":40"));
    assert!(json.contains("\"preferences_version\":5"));
    assert!(json.contains("\"plan_gate\":\"gated\""));
    assert!(json.contains("\"chat_streaming\":\"token\""));
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
    // A config written before the setting existed reads as the new default.
    assert_eq!(cfg.chat_streaming, ChatStreamMode::Paragraph);
}

#[test]
fn invalid_report_authoring_preferences_are_rejected() {
    let invalid = ReportAuthoringPreferences {
        max_converse_calls: 0,
        ..ReportAuthoringPreferences::default()
    };

    assert!(invalid.validate().is_err());
}

/// The PIN hash is a credential for this computer. Syncing it would put it in
/// the bucket it exists to protect, and would carry a lock a clinician set on
/// one machine onto every other one they use.
#[test]
fn the_pin_hash_never_reaches_the_synced_preferences() {
    let mut cfg = sample_config();
    cfg.security.auto_lock_enabled = true;
    cfg.security.pin_hash = Some(
        "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA"
            .to_string()
            .into(),
    );

    let json = serde_json::to_string(&SyncedPreferences::from_config(&cfg)).expect("serialize");

    assert!(!json.contains("security"), "{json}");
    assert!(!json.contains("pin_hash"), "{json}");
    assert!(!json.contains("argon2"), "{json}");
}

/// A synced-preferences overlay arriving from another machine must not be
/// able to reach the lock settings, however it was written.
#[test]
fn applying_synced_preferences_leaves_the_lock_alone() {
    let mut cfg = sample_config();
    cfg.security.auto_lock_enabled = true;
    cfg.security.auto_lock_timeout_minutes = 15;
    cfg.security.pin_hash = Some("$argon2id$v=19$hash".to_string().into());

    let synced = SyncedPreferences::from_config(&sample_config());
    synced.apply_to_config(&mut cfg);

    assert!(cfg.security.auto_lock_enabled);
    assert_eq!(cfg.security.auto_lock_timeout_minutes, 15);
    assert!(cfg.security.pin_set());
}

/// `Sensitive` is what makes a stray `{:?}` of the whole config harmless. The
/// hash is one debug-print away from a support export otherwise.
#[test]
fn debug_printing_the_config_does_not_print_the_pin_hash() {
    let mut cfg = sample_config();
    cfg.security.pin_hash = Some(
        "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA"
            .to_string()
            .into(),
    );

    let rendered = format!("{cfg:?}");

    assert!(!rendered.contains("argon2"), "{rendered}");
    assert!(rendered.contains("[redacted]"), "{rendered}");
}
