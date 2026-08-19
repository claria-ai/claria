//! Tests for PIN hashing and lock policy helpers.

use std::time::Duration;

use claria_desktop::security::{
    LockRuntime, MAX_TIMEOUT_MINUTES, MIN_TIMEOUT_MINUTES, backoff_duration,
    biometric_failure_message, hash_pin, idle_timeout, validate_pin, validate_timeout_minutes,
    verify_pin, wall_clock_jumped,
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
        let message = biometric_failure_message(raw).unwrap_or_else(|| panic!("{raw} was silenced"));
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
