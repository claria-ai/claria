//! Tests for PIN hashing and lock policy helpers.

use std::time::Duration;

use claria_desktop::{
    config::{self, CURRENT_VERSION},
    security::{
        LockRuntime, MAX_TIMEOUT_MINUTES, MIN_TIMEOUT_MINUTES, backoff_duration,
        biometric_failure_message, hash_pin, idle_timeout, validate_pin, validate_timeout_minutes,
        verify_pin, wall_clock_jumped,
    },
};

#[test]
fn pin_validation() {
    assert!(validate_pin("123456").is_ok());
    assert!(validate_pin("123456789012").is_ok());

    assert!(validate_pin("12345").is_err()); // too short
    assert!(validate_pin("1234567890123").is_err()); // too long
    assert!(validate_pin("12345a").is_err()); // non-digit
    assert!(validate_pin("12 456").is_err()); // whitespace
    assert!(validate_pin("").is_err());
    assert!(validate_pin("١٢٣٤٥٦").is_err()); // non-ASCII digits
}

#[test]
fn hash_and_verify_round_trip() {
    let hash = hash_pin("482913").unwrap();

    assert!(hash.starts_with("$argon2id$"));
    assert!(verify_pin("482913", &hash).unwrap());
    assert!(!verify_pin("482914", &hash).unwrap());
    assert!(!verify_pin("", &hash).unwrap());
}

#[test]
fn hashes_are_salted() {
    let a = hash_pin("482913").unwrap();
    let b = hash_pin("482913").unwrap();
    assert_ne!(a, b);
}

#[test]
fn verify_rejects_malformed_hash() {
    assert!(verify_pin("482913", "not-a-phc-string").is_err());
}

#[test]
fn backoff_schedule() {
    assert_eq!(backoff_duration(0), None);
    assert_eq!(backoff_duration(1), None);
    assert_eq!(backoff_duration(2), None);
    assert_eq!(backoff_duration(3), Some(Duration::from_secs(5)));
    assert_eq!(backoff_duration(4), Some(Duration::from_secs(10)));
    assert_eq!(backoff_duration(5), Some(Duration::from_secs(20)));
    assert_eq!(backoff_duration(9), Some(Duration::from_secs(300))); // cap
    assert_eq!(backoff_duration(u32::MAX), Some(Duration::from_secs(300))); // no overflow
}

#[test]
fn wall_clock_jump_detection() {
    let tick = Duration::from_secs(10);
    let slack = Duration::from_secs(60);
    let t0 = jiff::Timestamp::UNIX_EPOCH;

    // Normal tick cadence, even a slow one, is not a jump.
    assert!(!wall_clock_jumped(
        t0,
        t0 + Duration::from_secs(10),
        tick,
        slack
    ));
    assert!(!wall_clock_jumped(
        t0,
        t0 + Duration::from_secs(70),
        tick,
        slack
    ));

    // Past tick + slack means the machine slept.
    assert!(wall_clock_jumped(
        t0,
        t0 + Duration::from_secs(71),
        tick,
        slack
    ));
    assert!(wall_clock_jumped(
        t0,
        t0 + Duration::from_secs(3600),
        tick,
        slack
    ));

    // Clock moving backwards is not a jump.
    assert!(!wall_clock_jumped(
        t0 + Duration::from_secs(100),
        t0,
        tick,
        slack
    ));
}

#[test]
fn lock_runtime_starts_unlocked() {
    let rt = LockRuntime::default();
    assert!(!rt.locked());
    assert_eq!(rt.failed_attempts(), 0);
    assert_eq!(rt.backoff_remaining_secs(), None);
}

#[test]
fn lock_runtime_idle_expiry() {
    let mut rt = LockRuntime::default();
    assert!(rt.idle_expired(Duration::ZERO));
    assert!(!rt.idle_expired(Duration::from_secs(3600)));

    // A locked session never reports idle expiry.
    rt.lock();
    assert!(!rt.idle_expired(Duration::ZERO));
}

#[test]
fn lock_runtime_activity_ignored_while_locked() {
    let mut rt = LockRuntime::default();
    rt.lock();
    rt.note_activity();
    rt.unlock();
    // Unlock resets the idle timer, so nothing observable — this test
    // documents that note_activity while locked doesn't panic or unlock.
    assert!(!rt.locked());
}

#[test]
fn lock_runtime_failure_and_backoff() {
    let mut rt = LockRuntime::default();
    rt.lock();

    rt.register_failure();
    rt.register_failure();
    assert_eq!(rt.failed_attempts(), 2);
    assert_eq!(rt.backoff_remaining_secs(), None);

    rt.register_failure();
    assert_eq!(rt.failed_attempts(), 3);
    let remaining = rt.backoff_remaining_secs().unwrap();
    assert!((1..=5).contains(&remaining));

    // Unlock clears failures and backoff.
    rt.unlock();
    assert_eq!(rt.failed_attempts(), 0);
    assert_eq!(rt.backoff_remaining_secs(), None);
}

#[test]
fn timeout_validation() {
    assert!(validate_timeout_minutes(MIN_TIMEOUT_MINUTES).is_ok());
    assert!(validate_timeout_minutes(5).is_ok());
    assert!(validate_timeout_minutes(MAX_TIMEOUT_MINUTES).is_ok());

    assert!(validate_timeout_minutes(0).is_err());
    assert!(validate_timeout_minutes(MAX_TIMEOUT_MINUTES + 1).is_err());
}

/// A stored timeout is clamped rather than refused. A config that somehow
/// carries zero would otherwise lock the app on every watcher tick, and one
/// that carries `u32::MAX` would overflow the multiplication into minutes.
#[test]
fn a_stored_timeout_outside_the_range_is_clamped_not_obeyed() {
    assert_eq!(idle_timeout(0), Duration::from_secs(60));
    assert_eq!(idle_timeout(5), Duration::from_secs(300));
    assert_eq!(
        idle_timeout(u32::MAX),
        Duration::from_secs(u64::from(MAX_TIMEOUT_MINUTES) * 60)
    );
}

/// The panel's cancel button is labelled "Use PIN". Pressing it — someone
/// explicitly asking to type their PIN — arrives as `userCancel`, and must
/// not come back as an error to render at them.
#[test]
fn a_dismissed_biometric_panel_is_not_an_error() {
    for raw in [
        "[userCancel] - Authentication canceled.",
        "[userFallback] - Fallback authentication mechanism selected.",
        "[appCancel] - Authentication was canceled by application.",
        "[systemCancel] - Authentication was canceled by system.",
    ] {
        assert_eq!(biometric_failure_message(raw), None, "{raw}");
    }
}

/// Whatever else the panel says, the clinician gets a sentence. A `LAError`
/// case name in a red box on a lock screen is the bug this mapping exists to
/// prevent, so the assertion is about the shape of every answer, not just the
/// cases named here.
#[test]
fn every_real_biometric_failure_reads_as_prose() {
    let raws = [
        "[authenticationFailed] - Biometry is locked out.",
        "[biometryLockout] - Biometry is locked out.",
        "[biometryNotEnrolled] - Biometry is not enrolled.",
        "[biometryNotAvailable] - Biometry is not available.",
        "[passcodeNotSet] - Passcode not set.",
        "[invalidContext] - Context is invalid.",
        "[notInteractive] - Interaction is not allowed.",
        // Windows' own catch-all, and a case the plugin has not invented yet.
        "[internalError] - Failed to request user verification: Error(...)",
        "[somethingNewInTheNextRelease] - who knows",
        // Not every variant of the plugin's error even carries a code.
        "failed to deserialize response: expected value",
        "",
    ];

    for raw in raws {
        let message =
            biometric_failure_message(raw).unwrap_or_else(|| panic!("{raw} was silenced"));
        assert!(
            !message.contains('[') && !message.contains(']'),
            "{raw} leaked a code: {message}"
        );
        assert!(
            message.ends_with("Enter your PIN instead."),
            "{raw} left no way forward: {message}"
        );
        // The panel's own text is Apple's or Microsoft's, not ours.
        assert!(!raw.contains(message), "{raw} was passed through verbatim");
    }
}

// ── The lock survives a config the rest of the build cannot load ─────────
//
// `lock_at_startup` used to read the whole config and take any failure as
// "not armed", so a machine with auto-lock on booted unlocked. That also
// quietly undid the backoff reasoning in `design/app-lock.md`, which rests on
// a restart coming back locked. These pin the narrow read that replaced it.

/// Whether `load_config` refuses these contents outright — the premise each
/// of these tests rests on.
fn whole_config_fails_to_load(json: &str) -> bool {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.json");
    std::fs::write(&path, json).expect("write");
    config::load_config_at(&path).is_err()
}

/// A config with a PIN set and auto-lock on, at `config_version`.
fn armed_config_json(config_version: u32) -> String {
    let hash = hash_pin("123456").expect("hash a PIN");
    format!(
        r#"{{
            "config_version": {config_version},
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": {{ "type": "default_chain" }},
            "security": {{
                "auto_lock_enabled": true,
                "auto_lock_timeout_minutes": 10,
                "biometric_unlock_enabled": false,
                "pin_hash": "{hash}"
            }}
        }}"#
    )
}

#[test]
fn a_config_too_new_to_load_still_locks_the_app() {
    let json = armed_config_json(CURRENT_VERSION + 1);
    assert!(
        whole_config_fails_to_load(&json),
        "the premise: this build cannot load the config as a whole"
    );

    let settings = config::parse_security_settings(&json).expect("the lock settings still read");
    assert!(
        settings.armed(),
        "a machine whose PIN is set must still come back locked"
    );
}

/// The unlock has to work on the same machine, or the lock is a door with no
/// key. `settings()` and `lock_at_startup` both go through the narrow read,
/// so the PIN the overlay checks against is the one on disk.
#[test]
fn the_pin_survives_a_config_too_new_to_load() {
    let json = armed_config_json(CURRENT_VERSION + 1);
    let settings = config::parse_security_settings(&json).expect("the lock settings still read");

    let hash = settings.pin_hash.as_ref().expect("a PIN is stored");
    assert!(
        verify_pin("123456", hash.reveal()).expect("verify"),
        "the stored PIN must still answer the lock screen"
    );
    assert!(!verify_pin("999999", hash.reveal()).expect("verify"));
}

/// The other way a whole-config load fails: it parses, but a validated
/// section is out of range. The lock has nothing to do with those limits.
#[test]
fn a_config_whose_limits_no_longer_validate_still_locks_the_app() {
    let hash = hash_pin("123456").expect("hash a PIN");
    let json = format!(
        r#"{{
            "config_version": {CURRENT_VERSION},
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": {{ "type": "default_chain" }},
            "report_authoring": {{ "max_tool_rounds": 0 }},
            "security": {{
                "auto_lock_enabled": true,
                "auto_lock_timeout_minutes": 10,
                "pin_hash": "{hash}"
            }}
        }}"#
    );

    assert!(
        whole_config_fails_to_load(&json),
        "the premise: an out-of-range limit fails the whole load"
    );
    assert!(
        config::parse_security_settings(&json)
            .expect("the lock settings still read")
            .armed()
    );
}

/// The one case that still fails open, and deliberately: JSON that will not
/// parse holds no PIN hash, so locking would mean an overlay nothing can
/// dismiss — a clinician shut out of their own records.
#[test]
fn unparseable_json_cannot_lock_because_it_holds_no_pin() {
    assert!(config::parse_security_settings("{ not json").is_none());
}

/// A config written before the lock existed has no `security` key at all.
/// Unarmed is the right answer, and it must not be mistaken for a failure.
#[test]
fn a_config_without_a_security_section_reads_as_unarmed() {
    let json = format!(
        r#"{{
            "config_version": {CURRENT_VERSION},
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": {{ "type": "default_chain" }}
        }}"#
    );

    let settings = config::parse_security_settings(&json).unwrap_or_default();
    assert!(!settings.armed());
    assert!(!settings.pin_set());
}

/// Auto-lock without a PIN is never armed — the invariant the whole feature
/// rests on, restated against the narrow read so it cannot drift from it.
#[test]
fn auto_lock_without_a_pin_is_not_armed() {
    let json = format!(
        r#"{{
            "config_version": {CURRENT_VERSION},
            "region": "us-east-1",
            "system_name": "test",
            "account_id": "123456789012",
            "created_at": "1970-01-01T00:00:00Z",
            "credentials": {{ "type": "default_chain" }},
            "security": {{ "auto_lock_enabled": true }}
        }}"#
    );

    let settings = config::parse_security_settings(&json).expect("reads");
    assert!(settings.auto_lock_enabled);
    assert!(!settings.pin_set());
    assert!(!settings.armed(), "no PIN means no lock, however it is set");
}

/// An older config is migrated before the subtree is read, so a field the
/// migration chain moved is read the way the rest of the config reads it.
#[test]
fn an_older_config_is_migrated_before_its_lock_settings_are_read() {
    let json = armed_config_json(1);
    let settings = config::parse_security_settings(&json).expect("reads");
    assert!(settings.armed());
}

/// The narrow read goes through a file like the startup path does.
#[test]
fn the_lock_settings_load_from_a_file_on_disk() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.json");
    std::fs::write(&path, armed_config_json(CURRENT_VERSION + 1)).expect("write");

    assert!(
        config::load_security_settings_at(&path)
            .expect("reads")
            .armed()
    );
    assert!(
        config::load_security_settings_at(&dir.path().join("nope.json")).is_none(),
        "a config that is not there is not a locked machine"
    );
}
