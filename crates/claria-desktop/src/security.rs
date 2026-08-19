//! PIN hashing and lock policy for the auto-lock feature.
//!
//! Pure logic only — the Tauri command layer owns the wiring (state, events,
//! biometric prompts). Kept in the lib so integration tests can exercise it
//! without a Tauri runtime.
//!
//! Every bound the feature has — PIN length, idle-timeout range, the backoff
//! schedule — lives here rather than at the call sites that enforce them, so
//! the command layer, the config layer, and the tests cannot disagree about
//! what the policy is.

use std::time::{Duration, Instant};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

pub const MIN_PIN_LEN: usize = 6;
pub const MAX_PIN_LEN: usize = 12;

/// Idle timeout offered on first setup.
pub const DEFAULT_TIMEOUT_MINUTES: u32 = 5;
/// Shortest idle timeout accepted. Anything under a minute locks mid-sentence.
pub const MIN_TIMEOUT_MINUTES: u32 = 1;
/// Longest idle timeout accepted. Past two hours the setting is theatre.
pub const MAX_TIMEOUT_MINUTES: u32 = 120;

/// Free attempts before backoff kicks in.
const FREE_ATTEMPTS: u32 = 3;
/// First backoff window; doubles per subsequent failure.
const BASE_BACKOFF_SECS: u64 = 5;
const MAX_BACKOFF_SECS: u64 = 300;

pub fn validate_pin(pin: &str) -> Result<(), String> {
    if !(MIN_PIN_LEN..=MAX_PIN_LEN).contains(&pin.len()) {
        return Err(format!(
            "PIN must be {MIN_PIN_LEN}–{MAX_PIN_LEN} digits long"
        ));
    }
    if !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err("PIN must contain only digits".to_string());
    }
    Ok(())
}

pub fn validate_timeout_minutes(minutes: u32) -> Result<(), String> {
    if !(MIN_TIMEOUT_MINUTES..=MAX_TIMEOUT_MINUTES).contains(&minutes) {
        return Err(format!(
            "Lock timeout must be between {MIN_TIMEOUT_MINUTES} and {MAX_TIMEOUT_MINUTES} minutes"
        ));
    }
    Ok(())
}

/// The idle window a stored timeout means, clamped into the accepted range.
///
/// Clamping rather than validating on load is deliberate: a config that has
/// somehow acquired a zero or absurd timeout should still open the app, and
/// should still lock it. Refusing the load would strand the clinician outside
/// their own records over a setting.
pub fn idle_timeout(minutes: u32) -> Duration {
    Duration::from_secs(u64::from(minutes.clamp(MIN_TIMEOUT_MINUTES, MAX_TIMEOUT_MINUTES)) * 60)
}

/// Hash a PIN into an argon2id PHC string (parameters are self-describing,
/// so defaults can change without invalidating stored hashes).
pub fn hash_pin(pin: &str) -> eyre::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(pin.as_bytes(), &salt)
        .map_err(|e| eyre::eyre!("failed to hash PIN: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_pin(pin: &str, phc_hash: &str) -> eyre::Result<bool> {
    let parsed =
        PasswordHash::new(phc_hash).map_err(|e| eyre::eyre!("stored PIN hash is invalid: {e}"))?;
    Ok(Argon2::default()
        .verify_password(pin.as_bytes(), &parsed)
        .is_ok())
}

/// Backoff imposed after `failed_attempts` consecutive failures, or `None`
/// while still within the free-attempt budget. 3 failures → 5 s, doubling
/// per failure, capped at 5 minutes.
pub fn backoff_duration(failed_attempts: u32) -> Option<Duration> {
    if failed_attempts < FREE_ATTEMPTS {
        return None;
    }
    let doublings = (failed_attempts - FREE_ATTEMPTS).min(63);
    let secs = BASE_BACKOFF_SECS
        .checked_shl(doublings)
        .unwrap_or(MAX_BACKOFF_SECS)
        .min(MAX_BACKOFF_SECS);
    Some(Duration::from_secs(secs))
}

/// What to show a clinician when the biometric panel comes back refusing.
///
/// `None` means "say nothing": the panel was dismissed rather than failed.
/// Cancelling Touch ID is how someone chooses the PIN field, and the lock
/// screen already invites that, so rendering it as a red error box tells them
/// off for doing the intended thing.
///
/// Everything else is mapped to a sentence. The plugin's own `Display` is
/// `[userCancel] - Authentication canceled.` — a `LAError` case name and an
/// Apple string — and that must never reach the overlay. The raw text is
/// matched here rather than the plugin's error enum because
/// `tauri-plugin-biometry` re-exports only `Error` and `Result`; its
/// `ErrorResponse`, which holds the code as a field, is private. Callers log
/// the raw string so a support export keeps the diagnostic.
pub fn biometric_failure_message(raw: &str) -> Option<&'static str> {
    let code = raw
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map(|(code, _)| code)
        .unwrap_or_default();

    Some(match code {
        // Deliberate dismissals. `userFallback` is the panel's own "Use
        // Password…" button: also a decision to type something instead, and
        // the thing they can type here is the PIN.
        "userCancel" | "userFallback" | "appCancel" | "systemCancel" => return None,
        "authenticationFailed" => "Not recognised. Enter your PIN instead.",
        "biometryLockout" => "Too many biometric attempts. Enter your PIN instead.",
        "biometryNotEnrolled" => {
            "No biometric is enrolled on this computer. Enter your PIN instead."
        }
        "biometryNotAvailable" => "Biometric unlock is unavailable. Enter your PIN instead.",
        "passcodeNotSet" => {
            "Biometric unlock needs a passcode on this computer. Enter your PIN instead."
        }
        // `invalidContext`, `notInteractive`, `unknown`, and anything the
        // plugin adds later. A case nobody anticipated is exactly the one
        // that would otherwise leak its code into the UI.
        _ => "Biometric unlock could not run. Enter your PIN instead.",
    })
}

/// True when the wall clock advanced far beyond one watcher tick — the
/// machine was asleep (or the clock jumped), so the session should lock.
/// Monotonic clocks behave differently across platforms during sleep;
/// wall-clock deltas don't.
pub fn wall_clock_jumped(
    prev: jiff::Timestamp,
    now: jiff::Timestamp,
    expected_tick: Duration,
    slack: Duration,
) -> bool {
    let threshold = expected_tick + slack;
    now.duration_since(prev).as_secs() > threshold.as_secs() as i64
}

/// In-memory lock state. Lives in `DesktopState` behind a mutex; deliberately
/// not persisted so a fresh launch always derives its lock state from config.
pub struct LockRuntime {
    locked: bool,
    failed_attempts: u32,
    backoff_until: Option<Instant>,
    last_activity: Instant,
}

impl Default for LockRuntime {
    fn default() -> Self {
        Self {
            locked: false,
            failed_attempts: 0,
            backoff_until: None,
            last_activity: Instant::now(),
        }
    }
}

impl LockRuntime {
    pub fn locked(&self) -> bool {
        self.locked
    }

    pub fn failed_attempts(&self) -> u32 {
        self.failed_attempts
    }

    /// Record user activity. Ignored while locked — activity on the lock
    /// screen must not feed the idle timer that decides re-locking.
    pub fn note_activity(&mut self) {
        if !self.locked {
            self.last_activity = Instant::now();
        }
    }

    pub fn idle_expired(&self, timeout: Duration) -> bool {
        !self.locked && self.last_activity.elapsed() >= timeout
    }

    pub fn lock(&mut self) {
        self.locked = true;
    }

    /// Unlock and reset the failure counter and idle timer.
    pub fn unlock(&mut self) {
        self.locked = false;
        self.failed_attempts = 0;
        self.backoff_until = None;
        self.last_activity = Instant::now();
    }

    /// Record a failed unlock attempt and arm the resulting backoff window.
    pub fn register_failure(&mut self) {
        self.failed_attempts += 1;
        self.backoff_until = backoff_duration(self.failed_attempts).map(|d| Instant::now() + d);
    }

    /// Seconds until unlock attempts are accepted again, if in backoff.
    pub fn backoff_remaining_secs(&self) -> Option<u64> {
        let until = self.backoff_until?;
        let now = Instant::now();
        if now >= until {
            return None;
        }
        // Round up so the UI never shows "0 s remaining" on a rejected attempt.
        Some((until - now).as_secs().max(1))
    }
}
