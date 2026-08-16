//! `StorageError` reaches the console ring buffer and the rolling on-disk logs
//! through the desktop command boundary's `error = ?error`, so a record key's
//! client-chosen filename must not survive either rendering.

use claria_core::s3_keys;
use claria_storage::error::{LogSafeKey, StorageError};
use uuid::Uuid;

fn record_key(id: Uuid) -> String {
    s3_keys::client_record_file(id, "Jane Doe intake.pdf")
}

fn redacted(id: Uuid) -> String {
    format!("records/{id}/<file>")
}

#[test]
fn record_key_errors_display_without_the_filename() {
    let id = Uuid::new_v4();
    let key = record_key(id);
    let redacted = redacted(id);

    let errors = [
        StorageError::NotFound {
            key: key.as_str().into(),
        },
        StorageError::PreconditionFailed {
            key: key.as_str().into(),
        },
        StorageError::ConditionalRequestConflict {
            key: key.as_str().into(),
        },
        StorageError::InvalidState {
            key: key.as_str().into(),
            reason: "revision went backwards".to_string(),
        },
        StorageError::ObjectTooLarge {
            key: key.as_str().into(),
            actual_bytes: 9,
            max_bytes: 8,
        },
    ];

    for error in &errors {
        let rendered = error.to_string();
        assert!(
            rendered.contains(&redacted),
            "Display lost the redacted key: {rendered}"
        );
        assert!(
            !rendered.contains("Jane Doe"),
            "Display leaked the filename: {rendered}"
        );
    }
}

#[test]
fn record_key_errors_debug_without_the_filename() {
    let id = Uuid::new_v4();
    let key = record_key(id);
    let redacted = redacted(id);

    let errors = [
        StorageError::NotFound {
            key: key.as_str().into(),
        },
        StorageError::InvalidState {
            key: key.as_str().into(),
            reason: "revision went backwards".to_string(),
        },
        StorageError::ObjectTooLarge {
            key: key.as_str().into(),
            actual_bytes: 9,
            max_bytes: 8,
        },
    ];

    for error in &errors {
        let rendered = format!("{error:?}");
        assert!(
            rendered.contains(&redacted),
            "Debug lost the redacted key: {rendered}"
        );
        assert!(
            !rendered.contains("Jane Doe"),
            "Debug leaked the filename: {rendered}"
        );
    }
}

#[test]
fn app_generated_keys_render_unchanged() {
    for key in [
        s3_keys::PREFERENCES,
        s3_keys::PROVISIONER_STATE,
        "report-authoring/6f0f0d5e-0000-4000-8000-000000000000/workspace.json",
    ] {
        let error = StorageError::NotFound { key: key.into() };
        let rendered = error.to_string();
        assert!(
            rendered.contains(key),
            "non-record key was altered: {rendered}"
        );
        assert_eq!(format!("{error:?}"), format!("NotFound {{ key: {key:?} }}"));
    }
}

#[test]
fn the_unredacted_key_stays_available_to_code() {
    let id = Uuid::new_v4();
    let key = record_key(id);
    let wrapped = LogSafeKey::from(key.as_str());

    assert_eq!(wrapped.as_str(), key);
    assert_ne!(wrapped.to_string(), key);
}
