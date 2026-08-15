//! What a clinician is told when the saved config cannot be loaded.
//!
//! "Complete setup first" is reserved for a genuinely absent config. Every
//! other failure has to carry its own reason, because that message invites
//! the user to re-run setup and overwrite the file.

use claria_desktop::config::{self, CURRENT_VERSION, SETUP_REQUIRED};

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
