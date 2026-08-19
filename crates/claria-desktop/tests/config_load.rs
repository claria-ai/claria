//! What a clinician is told when the saved config cannot be loaded.
//!
//! "Complete setup first" is reserved for a genuinely absent config. Every
//! other failure has to carry its own reason, because that message invites
//! the user to re-run setup and overwrite the file.

use claria_desktop::config::{self, CURRENT_VERSION, ChatStreamMode, PlanGateMode, SETUP_REQUIRED};

fn write_config(dir: &tempfile::TempDir, contents: &str) -> std::path::PathBuf {
    let path = dir.path().join("config.json");
    std::fs::write(&path, contents).expect("write test config");
    path
}

#[test]
fn missing_config_asks_the_user_to_complete_setup() {
    let dir = tempfile::tempdir().expect("temp dir");
    let error = config::load_config_at(&dir.path().join("config.json"))
        .expect_err("a missing config cannot load");

    assert_eq!(error.to_string(), SETUP_REQUIRED);
}

#[test]
fn config_newer_than_the_build_surfaces_the_version_mismatch() {
    let dir = tempfile::tempdir().expect("temp dir");
    let newer = CURRENT_VERSION + 1;
    let path = write_config(
        &dir,
        &format!(
            r#"{{
                "config_version": {newer},
                "region": "us-east-1",
                "system_name": "test",
                "account_id": "123456789012",
                "created_at": "1970-01-01T00:00:00Z",
                "credentials": {{ "type": "default_chain" }}
            }}"#
        ),
    );

    let error = config::load_config_at(&path).expect_err("a future config cannot load");
    let message = error.to_string();

    assert!(
        message.contains(&format!(
            "config_version {newer} is newer than this build supports ({CURRENT_VERSION})"
        )),
        "expected the version mismatch, got: {message}"
    );
    assert!(
        message.contains("Please update Claria"),
        "expected the update instruction, got: {message}"
    );
    assert_ne!(message, SETUP_REQUIRED);
}

#[test]
fn corrupt_config_surfaces_the_parse_failure() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(&dir, "{ not json");

    let error = config::load_config_at(&path).expect_err("a corrupt config cannot load");

    assert_ne!(error.to_string(), SETUP_REQUIRED);
}

#[test]
fn an_existing_config_still_loads_and_migrates() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(
        &dir,
        r#"{
            "config_version": 0,
            "region": "us-east-1",
            "system_name": "test",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": { "type": "default_chain" }
        }"#,
    );

    let config = config::load_config_at(&path).expect("a v0 config migrates forward");

    assert_eq!(config.region, "us-east-1");
    // The migrated form is written back, so the next load skips the chain.
    let (reloaded, on_disk_version) = config::read_config_at(&path)
        .expect("reread")
        .expect("present");
    assert_eq!(on_disk_version, CURRENT_VERSION);
    assert_eq!(reloaded.system_name, "test");
}

/// A config written before the drafting pipeline existed comes forward with
/// the gate on. Starting to draft against a plan nobody has seen is the one
/// behaviour an existing install must not silently acquire.
#[test]
fn a_v10_config_migrates_to_v11_with_the_plan_gate_on() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(
        &dir,
        r#"{
            "config_version": 10,
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": { "type": "default_chain" },
            "preferred_model_id": "us.anthropic.claude-opus-4-6-v1",
            "chat_streaming": "token"
        }"#,
    );

    let config = config::load_config_at(&path).expect("a v10 config migrates forward");

    assert_eq!(config.draft_pipeline.plan_gate, PlanGateMode::Gated);
    assert_eq!(config.draft_pipeline.planner_model_id, None);
    assert_eq!(config.draft_pipeline.reviewer_model_id, None);
    // Untouched settings survive the migration.
    assert_eq!(config.chat_streaming, ChatStreamMode::Token);
    assert_eq!(
        config.preferred_model_id.as_deref(),
        Some("us.anthropic.claude-opus-4-6-v1")
    );

    let (_, on_disk_version) = config::read_config_at(&path)
        .expect("reread")
        .expect("present");
    assert_eq!(on_disk_version, CURRENT_VERSION);
}

#[test]
fn a_v11_config_migrates_forward_onto_todays_waits() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(
        &dir,
        r#"{
            "config_version": 11,
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": { "type": "default_chain" },
            "report_authoring": {
                "max_tool_rounds": 12,
                "max_converse_calls": 13,
                "max_tool_uses_per_response": 16,
                "max_retained_turns": 30
            }
        }"#,
    );

    let config = config::load_config_at(&path).expect("a v11 config migrates forward");

    // An install that never said anything about the waits lands on today's
    // defaults, not on the ones that were compiled in when it was written.
    assert_eq!(config.report_authoring.writer_first_frame_timeout_secs, 180);
    assert_eq!(config.report_authoring.writer_idle_timeout_secs, 300);
    assert_eq!(config.report_authoring.writer_max_output_tokens, 32_768);
    assert_eq!(
        config.report_authoring.analysis_first_frame_timeout_secs,
        300
    );
    assert_eq!(config.report_authoring.analysis_idle_timeout_secs, 600);
    // The guardrails the clinician had already chosen survive.
    assert_eq!(config.report_authoring.max_tool_rounds, 12);
    assert_eq!(config.report_authoring.max_retained_turns, 30);

    let (_, on_disk_version) = config::read_config_at(&path)
        .expect("reread")
        .expect("present");
    assert_eq!(on_disk_version, CURRENT_VERSION);
}

#[test]
fn a_raised_wait_survives_the_round_trip_into_the_pipeline() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(
        &dir,
        r#"{
            "config_version": 12,
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": { "type": "default_chain" },
            "report_authoring": {
                "writer_first_frame_timeout_secs": 420,
                "writer_idle_timeout_secs": 240,
                "writer_max_output_tokens": 65536,
                "analysis_first_frame_timeout_secs": 500,
                "analysis_idle_timeout_secs": 300
            }
        }"#,
    );

    let config = config::load_config_at(&path).expect("a v12 config loads");
    let limits = config.report_authoring.limits().expect("within range");

    assert_eq!(limits.stream_bounds().first_frame_secs(), 420);
    assert_eq!(limits.stream_bounds().idle_secs(), 240);
    assert_eq!(limits.writer_max_output_tokens(), 65_536);
    assert_eq!(
        limits.runtime().analysis_stream_bounds().first_frame_secs(),
        500
    );
    assert_eq!(limits.runtime().analysis_stream_bounds().idle_secs(), 300);
}

#[test]
fn a_wait_past_the_ceiling_is_refused_before_it_reaches_bedrock() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(
        &dir,
        r#"{
            "config_version": 12,
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": { "type": "default_chain" },
            "report_authoring": { "writer_first_frame_timeout_secs": 6000 }
        }"#,
    );

    // Refused at load, not at call time: a wait nobody can honour must not
    // reach the point where it would fail a clinician's draft instead.
    let error = config::load_config_at(&path).expect_err("6000 seconds is past the ceiling");
    let message = error.to_string();
    assert!(
        message.contains(&claria::MAX_CONFIGURABLE_TIMEOUT_SECS.to_string()),
        "{message}"
    );
}

/// The default change alone reaches nobody: v12 wrote the four waits into
/// every existing config as literal numbers, so the migration is what
/// actually delivers the longer waits to the installed base.
#[test]
fn a_v12_config_still_on_the_old_waits_is_lifted_onto_the_new_ones() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(
        &dir,
        r#"{
            "config_version": 12,
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": { "type": "default_chain" },
            "report_authoring": {
                "writer_first_frame_timeout_secs": 90,
                "writer_idle_timeout_secs": 60,
                "writer_max_output_tokens": 32768,
                "analysis_first_frame_timeout_secs": 120,
                "analysis_idle_timeout_secs": 90
            }
        }"#,
    );

    let config = config::load_config_at(&path).expect("a v12 config migrates forward");

    assert_eq!(config.report_authoring.writer_first_frame_timeout_secs, 180);
    assert_eq!(config.report_authoring.writer_idle_timeout_secs, 300);
    assert_eq!(
        config.report_authoring.analysis_first_frame_timeout_secs,
        300
    );
    assert_eq!(config.report_authoring.analysis_idle_timeout_secs, 600);
    // Not a wait, and not swept up by a migration that only knows about
    // waits.
    assert_eq!(config.report_authoring.writer_max_output_tokens, 32_768);
}

/// A number a clinician typed is theirs, including one they raised to less
/// than the new default. The migration lifts what was never chosen, not
/// what was chosen and happens to be lower than we would pick now.
#[test]
fn a_wait_the_clinician_chose_survives_the_v14_migration() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(
        &dir,
        r#"{
            "config_version": 12,
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": { "type": "default_chain" },
            "report_authoring": {
                "writer_first_frame_timeout_secs": 90,
                "writer_idle_timeout_secs": 45,
                "analysis_first_frame_timeout_secs": 200,
                "analysis_idle_timeout_secs": 150
            }
        }"#,
    );

    let config = config::load_config_at(&path).expect("a v12 config migrates forward");

    // Untouched, so lifted.
    assert_eq!(config.report_authoring.writer_first_frame_timeout_secs, 180);
    // Deliberately below the old default, deliberately below the new one,
    // and left exactly where it was put.
    assert_eq!(config.report_authoring.writer_idle_timeout_secs, 45);
    assert_eq!(
        config.report_authoring.analysis_first_frame_timeout_secs,
        200
    );
    assert_eq!(config.report_authoring.analysis_idle_timeout_secs, 150);
}

/// Auto-lock arrives off. An existing install that gained a lock it never
/// configured would be one a clinician cannot open — there is no PIN to type.
#[test]
fn a_v12_config_migrates_to_v13_with_auto_lock_off() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(
        &dir,
        r#"{
            "config_version": 12,
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": { "type": "default_chain" },
            "chat_streaming": "token"
        }"#,
    );

    let config = config::load_config_at(&path).expect("a v12 config migrates forward");

    assert!(!config.security.auto_lock_enabled);
    assert!(!config.security.pin_set());
    assert!(!config.security.armed());
    assert!(!config.security.biometric_unlock_enabled);
    assert_eq!(
        config.security.auto_lock_timeout_minutes,
        claria_desktop::security::DEFAULT_TIMEOUT_MINUTES
    );
    // Untouched settings survive the migration.
    assert_eq!(config.chat_streaming, ChatStreamMode::Token);

    let (_, on_disk_version) = config::read_config_at(&path)
        .expect("reread")
        .expect("present");
    assert_eq!(on_disk_version, CURRENT_VERSION);
}

/// A configured lock survives the round trip through disk. The hash is the
/// only copy of the PIN that exists, so losing it on a save is losing the
/// clinician's way back in.
#[test]
fn a_configured_lock_survives_a_save_and_reload() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_config(
        &dir,
        r#"{
            "config_version": 13,
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": { "type": "default_chain" },
            "security": {
                "auto_lock_enabled": true,
                "auto_lock_timeout_minutes": 15,
                "biometric_unlock_enabled": true,
                "pin_hash": "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA"
            }
        }"#,
    );

    let config = config::load_config_at(&path).expect("a v13 config loads");

    assert!(config.security.armed());
    assert_eq!(config.security.auto_lock_timeout_minutes, 15);
    assert!(config.security.biometric_unlock_enabled);
    assert_eq!(
        config
            .security
            .pin_hash
            .as_ref()
            .map(|hash| hash.reveal().as_str()),
        Some("$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA")
    );
}
