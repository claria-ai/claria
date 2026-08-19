//! The session lock: auto-lock on idle or wake, PIN unlock, biometric unlock.
//!
//! This is walk-away protection. It hides an unattended screen from whoever
//! walks past it; it is not a defence against someone sitting down with the
//! machine and a debugger. The lock is one process-wide flag in
//! [`claria_desktop::security::LockRuntime`], broadcast to every window, and
//! the overlay the frontend raises over it.
//!
//! Two rules shape everything below:
//!
//! - **The PIN never leaves [`SecuritySettings`].** It is stored as an
//!   argon2id hash, it is never returned to the frontend, never logged, and
//!   never reaches an audit event's `details`. [`LockState`] reports only
//!   `pin_set`.
//! - **A rejected credential is an outcome, not an error.** Mistyping a PIN
//!   returns [`LockOutcome`] with `accepted: false`, so the lock screen can
//!   say "Incorrect PIN" in its own words instead of rendering a command
//!   name and a flattened error chain at a clinician.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use claria_storage::audit::actions;

use claria_desktop::{
    config::{self, SecuritySettings},
    security,
};

use super::{CommandContext, CommandError, run};
use crate::state::DesktopState;

/// How often the watcher looks at the idle timer and the wall clock.
const WATCHER_TICK: Duration = Duration::from_secs(10);

/// How far past one tick the wall clock may drift before the gap reads as
/// sleep rather than scheduling jitter.
const SLEEP_SLACK: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Everything the frontend is allowed to know about the lock.
///
/// The stored PIN appears here only as `pin_set`. There is deliberately no
/// field that could carry the hash, so no future edit can add one by
/// forgetting to redact.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LockState {
    /// Whether the overlay should be up.
    pub locked: bool,
    pub auto_lock_enabled: bool,
    pub auto_lock_timeout_minutes: u32,
    pub biometric_unlock_enabled: bool,
    /// Whether a PIN is stored. Never the PIN, never its hash.
    pub pin_set: bool,
    /// Consecutive failed unlock attempts since the last success.
    pub failed_attempts: u32,
    /// Seconds until attempts are accepted again, or `None` when they already
    /// are.
    pub backoff_remaining_seconds: Option<u32>,
}

/// Broadcast to every window whenever the lock changes.
///
/// Carries the whole state rather than a `locked` flag so a window that was
/// not the one making the change does not have to follow the event with a
/// round trip to find out what changed.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct LockStateChanged {
    pub state: LockState,
}

/// The result of a command that had to check a credential first.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LockOutcome {
    /// Whether the credential was accepted.
    pub accepted: bool,
    /// Lock state after the attempt, including any backoff it earned.
    pub state: LockState,
    /// Why the attempt could not be made, when there is something worth
    /// saying — a backoff window, a biometric lockout, an unenrolled sensor.
    /// `None` for a plain wrong PIN and for a prompt the user dismissed;
    /// both of those the UI phrases itself.
    pub error: Option<String>,
}

/// Which biometric this machine offers, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BiometryKind {
    TouchId,
    FaceId,
    WindowsHello,
    /// A sensor the platform did not name.
    Biometric,
    None,
}

/// Whether a fingerprint or face can answer the lock screen on this machine.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct BiometryAvailability {
    pub available: bool,
    pub kind: BiometryKind,
    /// Why biometrics are unavailable, when they are — no sensor, nothing
    /// enrolled, locked out after too many failures.
    pub error: Option<String>,
}

/// Why the session locked. A closed vocabulary because it reaches the audit
/// trail's `details`, which is console-safe by contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockReason {
    /// The idle timer expired.
    Idle,
    /// The wall clock jumped past a tick — the machine slept.
    Sleep,
    /// The app launched with auto-lock configured.
    Startup,
    /// The clinician asked for it.
    Manual,
}

impl LockReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Sleep => "sleep",
            Self::Startup => "startup",
            Self::Manual => "manual",
        }
    }
}

// ---------------------------------------------------------------------------
// Shared plumbing
// ---------------------------------------------------------------------------

/// The security settings in force: the in-memory config first, disk as the
/// fallback, defaults before setup has run.
///
/// Fails open on a config that cannot be parsed, because the alternative is
/// worse: a machine whose config no longer reads has no PIN hash to check
/// against, so failing closed would mean an overlay nothing can dismiss.
async fn settings(state: &State<'_, DesktopState>) -> Result<SecuritySettings, CommandError> {
    {
        let guard = state.config.lock().await;
        if let Some(cfg) = guard.as_ref() {
            return Ok(cfg.security.clone());
        }
    }
    if !config::has_config() {
        return Ok(SecuritySettings::default());
    }
    Ok(config::load_config()?.security)
}

/// Read the saved config, let `edit` change its lock settings, write it back,
/// and refresh the in-memory copy.
///
/// Deliberately not `apply_preferences_patch`: these settings are
/// machine-local and must never touch `_state/preferences.json`.
async fn update_settings(
    state: &State<'_, DesktopState>,
    edit: impl FnOnce(&mut SecuritySettings),
) -> Result<SecuritySettings, CommandError> {
    let mut cfg = config::load_config()?;
    edit(&mut cfg.security);
    config::save_config(&cfg)?;

    let settings = cfg.security.clone();
    let mut guard = state.config.lock().await;
    *guard = Some(cfg);
    Ok(settings)
}

/// The current lock state for `settings`.
fn snapshot(state: &State<'_, DesktopState>, settings: &SecuritySettings) -> LockState {
    let runtime = state.lock_runtime();
    LockState {
        locked: runtime.locked(),
        auto_lock_enabled: settings.auto_lock_enabled,
        auto_lock_timeout_minutes: settings.auto_lock_timeout_minutes,
        biometric_unlock_enabled: settings.biometric_unlock_enabled,
        pin_set: settings.pin_set(),
        failed_attempts: runtime.failed_attempts(),
        backoff_remaining_seconds: runtime
            .backoff_remaining_secs()
            .map(|seconds| seconds.min(u64::from(u32::MAX)) as u32),
    }
}

/// Send the current lock state to every window and hand it back to the caller.
///
/// The console window shares the main window's lock, so a change made in
/// Preferences has to reach it too.
fn publish(
    app: &AppHandle,
    state: &State<'_, DesktopState>,
    settings: &SecuritySettings,
) -> Result<LockState, CommandError> {
    let snapshot = snapshot(state, settings);
    LockStateChanged {
        state: snapshot.clone(),
    }
    .emit(app)
    .map_err(|error| CommandError::Msg(format!("failed to broadcast lock state: {error}")))?;
    Ok(snapshot)
}

/// File a session event on the durable S3 trail.
///
/// Best effort by design. The event being recorded has already happened —
/// refusing to lock because the receipt could not be written would be the
/// wrong trade in both directions. A machine that has no usable configuration
/// has no bucket to write to either, which is logged rather than surfaced.
async fn record_session_audit(
    state: &State<'_, DesktopState>,
    action: claria_storage::audit::Action,
    details: serde_json::Value,
) {
    match CommandContext::new(state).await {
        Ok(ctx) => {
            let event = ctx
                .audit_event(action, "session", ctx.cfg.system_name.clone())
                .with_details(details);
            ctx.record_audit(event).await;
        }
        Err(error) => tracing::warn!(
            audit.action = %action,
            error = %error,
            "session event was not written to the audit trail"
        ),
    }
}

/// Raise the lock, tell every window, then file the receipt.
///
/// A no-op when already locked. Refused when no PIN is stored: an overlay
/// with no credential that dismisses it is not protection, it is a clinician
/// locked out of their own records.
///
/// The order matters. The overlay goes up before the audit write, so the
/// screen is covered whether or not S3 answers.
pub(crate) async fn engage_lock(app: &AppHandle, reason: LockReason) -> Result<(), CommandError> {
    let state = app.state::<DesktopState>();
    let settings = settings(&state).await?;
    if !settings.pin_set() {
        return Err(CommandError::Msg(
            "Auto-lock is not set up on this computer".to_string(),
        ));
    }

    {
        let mut runtime = state.lock_runtime();
        if runtime.locked() {
            return Ok(());
        }
        runtime.lock();
    }

    publish(app, &state, &settings)?;
    record_session_audit(
        &state,
        actions::SESSION_LOCK,
        serde_json::json!({ "reason": reason.as_str() }),
    )
    .await;
    Ok(())
}

/// Drop the lock after a credential was accepted, tell every window, then
/// file the receipt.
async fn release_lock(
    app: &AppHandle,
    state: &State<'_, DesktopState>,
    settings: &SecuritySettings,
    method: &'static str,
) -> Result<LockState, CommandError> {
    state.lock_runtime().unlock();
    let snapshot = publish(app, state, settings)?;
    record_session_audit(
        state,
        actions::SESSION_UNLOCK,
        serde_json::json!({ "method": method }),
    )
    .await;
    Ok(snapshot)
}

/// Verify a PIN against a stored hash on a blocking thread.
///
/// argon2 is deliberately slow, which is the point of it and the reason this
/// cannot run on an async worker.
async fn verify(pin: String, hash: String) -> Result<bool, CommandError> {
    tokio::task::spawn_blocking(move || security::verify_pin(&pin, &hash))
        .await
        .map_err(|error| CommandError::Msg(format!("PIN check did not complete: {error}")))?
        .map_err(CommandError::from)
}

/// Hash a PIN on a blocking thread. Same reason as [`verify`].
async fn hash(pin: String) -> Result<String, CommandError> {
    tokio::task::spawn_blocking(move || security::hash_pin(&pin))
        .await
        .map_err(|error| CommandError::Msg(format!("PIN hashing did not complete: {error}")))?
        .map_err(CommandError::from)
}

/// The stored hash, or the reason there is nothing to check against.
fn stored_hash(settings: &SecuritySettings) -> Result<String, CommandError> {
    settings
        .pin_hash
        .as_ref()
        .map(|hash| hash.reveal().clone())
        .ok_or_else(|| CommandError::Msg("No PIN is set on this computer".to_string()))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_lock_state(state: State<'_, DesktopState>) -> Result<LockState, String> {
    run("get_lock_state", async {
        let settings = settings(&state).await?;
        Ok(snapshot(&state, &settings))
    })
    .await
}

/// Report that the clinician did something. Feeds the idle timer.
///
/// The frontend throttles these hard — this is a heartbeat, not an event
/// stream, and it must stay cheap enough to call from a pointer handler.
#[tauri::command]
#[specta::specta]
pub fn record_activity(state: State<'_, DesktopState>) {
    state.lock_runtime().note_activity();
}

#[tauri::command]
#[specta::specta]
pub async fn lock_session(app: AppHandle) -> Result<(), String> {
    run("lock_session", engage_lock(&app, LockReason::Manual)).await
}

/// Answer the lock screen with the PIN.
#[tauri::command]
#[specta::specta]
pub async fn unlock_with_pin(
    app: AppHandle,
    state: State<'_, DesktopState>,
    pin: String,
) -> Result<LockOutcome, String> {
    run("unlock_with_pin", async {
        let settings = settings(&state).await?;
        let stored = stored_hash(&settings)?;

        // Refuse before spending the argon2 verification, so a flood of
        // attempts cannot use the check itself as the workload. The guard is
        // released on its own line: `snapshot` takes the same non-reentrant
        // mutex, and leaning on scrutinee-temporary scoping for that would be
        // a deadlock one edition change away.
        let waiting = state.lock_runtime().backoff_remaining_secs();
        if let Some(seconds) = waiting {
            return Ok(LockOutcome {
                accepted: false,
                state: snapshot(&state, &settings),
                error: Some(format!("Too many attempts. Try again in {seconds} s.")),
            });
        }

        if verify(pin, stored).await? {
            let state_after = release_lock(&app, &state, &settings, "pin").await?;
            return Ok(LockOutcome {
                accepted: true,
                state: state_after,
                error: None,
            });
        }

        let (failed_attempts, backoff) = {
            let mut runtime = state.lock_runtime();
            runtime.register_failure();
            (runtime.failed_attempts(), runtime.backoff_remaining_secs())
        };
        let state_after = publish(&app, &state, &settings)?;
        record_session_audit(
            &state,
            actions::SESSION_UNLOCK_FAILED,
            serde_json::json!({
                "method": "pin",
                "failed_attempts": failed_attempts,
                "backoff_seconds": backoff,
            }),
        )
        .await;
        Ok(LockOutcome {
            accepted: false,
            state: state_after,
            error: None,
        })
    })
    .await
}

/// Answer the lock screen with Touch ID / Face ID / Windows Hello.
///
/// **Only ever called from an explicit press on the lock screen.** The panel
/// this raises is system-modal and always-on-top, and auto-lock fires on an
/// idle timer, so anything that invoked it on its own would drop an OS dialog
/// over whatever application the clinician was actually using. The frontend
/// checks focus before calling; this checks again immediately before the
/// panel goes up, because the window can lose focus in the gap.
///
/// A dismissed prompt comes back as `accepted: false` with no error — the
/// clinician chose the PIN field instead, which is not a failure and does not
/// count against the backoff budget. Only a prompt that actually rejected
/// them is audited.
#[tauri::command]
#[specta::specta]
pub async fn unlock_with_biometric(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<LockOutcome, String> {
    run("unlock_with_biometric", async {
        let settings = settings(&state).await?;
        if !settings.biometric_unlock_enabled {
            return Err(CommandError::Msg(
                "Biometric unlock is not enabled".to_string(),
            ));
        }
        // A biometric that could outlive the PIN would be a second credential
        // with no fallback behind it.
        let _ = stored_hash(&settings)?;

        // Nothing to tell the clinician here: they did not ask for this, and
        // whatever they *are* looking at should not learn that Claria wanted
        // the screen. Reported as a dismissal, which is what it is.
        if !app_is_frontmost(&app) {
            tracing::debug!("biometric unlock refused: no Claria window is focused");
            return Ok(LockOutcome {
                accepted: false,
                state: snapshot(&state, &settings),
                error: None,
            });
        }

        match prompt_for_biometric(app.clone()).await? {
            Ok(()) => {
                let state_after = release_lock(&app, &state, &settings, "biometric").await?;
                Ok(LockOutcome {
                    accepted: true,
                    state: state_after,
                    error: None,
                })
            }
            Err(failure) => {
                // The raw text is `[userCancel] - Authentication canceled.` —
                // a LocalAuthentication case name. It belongs in the log,
                // where a support export can find it, and nowhere near the
                // overlay.
                let raw = failure.to_string();
                tracing::warn!(error = %raw, "biometric unlock did not succeed");
                let message = security::biometric_failure_message(&raw);

                // Pressing the panel's "Use PIN" button arrives here as
                // `userCancel`. Someone asking to type their PIN has not
                // failed to unlock, so it is not audited as one.
                if message.is_some() {
                    record_session_audit(
                        &state,
                        actions::SESSION_UNLOCK_FAILED,
                        serde_json::json!({ "method": "biometric" }),
                    )
                    .await;
                }
                Ok(LockOutcome {
                    accepted: false,
                    state: snapshot(&state, &settings),
                    error: message.map(str::to_owned),
                })
            }
        }
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_biometry_status(app: AppHandle) -> Result<BiometryAvailability, String> {
    run("get_biometry_status", async { biometry_status(app).await }).await
}

/// Turn auto-lock on with a freshly chosen PIN.
#[tauri::command]
#[specta::specta]
pub async fn enable_auto_lock(
    app: AppHandle,
    state: State<'_, DesktopState>,
    pin: String,
    timeout_minutes: u32,
) -> Result<LockState, String> {
    run("enable_auto_lock", async {
        security::validate_pin(&pin)?;
        security::validate_timeout_minutes(timeout_minutes)?;

        let pin_hash = hash(pin).await?;
        let settings = update_settings(&state, |security| {
            security.auto_lock_enabled = true;
            security.auto_lock_timeout_minutes = timeout_minutes;
            security.pin_hash = Some(pin_hash.into());
        })
        .await?;

        // Arming restarts the idle clock, so the first window is a full one
        // rather than however long the Preferences screen had been open.
        state.lock_runtime().note_activity();

        let state_after = publish(&app, &state, &settings)?;
        record_session_audit(
            &state,
            actions::SESSION_AUTO_LOCK_ENABLE,
            serde_json::json!({ "timeout_minutes": timeout_minutes }),
        )
        .await;
        Ok(state_after)
    })
    .await
}

/// Turn auto-lock off and discard the stored PIN. Requires the PIN, so
/// walking up to an unlocked machine is not enough to remove the lock.
#[tauri::command]
#[specta::specta]
pub async fn disable_auto_lock(
    app: AppHandle,
    state: State<'_, DesktopState>,
    pin: String,
) -> Result<LockOutcome, String> {
    run("disable_auto_lock", async {
        let settings = settings(&state).await?;
        let stored = stored_hash(&settings)?;

        if !verify(pin, stored).await? {
            return Ok(LockOutcome {
                accepted: false,
                state: snapshot(&state, &settings),
                error: None,
            });
        }

        let settings = update_settings(&state, |security| {
            *security = SecuritySettings::default();
        })
        .await?;
        // Nothing can dismiss an overlay once the PIN behind it is gone.
        state.lock_runtime().unlock();

        let state_after = publish(&app, &state, &settings)?;
        record_session_audit(
            &state,
            actions::SESSION_AUTO_LOCK_DISABLE,
            serde_json::json!({}),
        )
        .await;
        Ok(LockOutcome {
            accepted: true,
            state: state_after,
            error: None,
        })
    })
    .await
}

/// Replace the unlock PIN. The current one is required.
#[tauri::command]
#[specta::specta]
pub async fn change_pin(
    app: AppHandle,
    state: State<'_, DesktopState>,
    current_pin: String,
    new_pin: String,
) -> Result<LockOutcome, String> {
    run("change_pin", async {
        security::validate_pin(&new_pin)?;
        let settings = settings(&state).await?;
        let stored = stored_hash(&settings)?;

        if !verify(current_pin, stored).await? {
            return Ok(LockOutcome {
                accepted: false,
                state: snapshot(&state, &settings),
                error: None,
            });
        }

        let pin_hash = hash(new_pin).await?;
        let settings = update_settings(&state, |security| {
            security.pin_hash = Some(pin_hash.into());
        })
        .await?;

        let state_after = publish(&app, &state, &settings)?;
        record_session_audit(&state, actions::SESSION_PIN_CHANGE, serde_json::json!({})).await;
        Ok(LockOutcome {
            accepted: true,
            state: state_after,
            error: None,
        })
    })
    .await
}

/// Change how long the app waits before locking itself.
#[tauri::command]
#[specta::specta]
pub async fn set_auto_lock_timeout(
    app: AppHandle,
    state: State<'_, DesktopState>,
    timeout_minutes: u32,
) -> Result<LockState, String> {
    run("set_auto_lock_timeout", async {
        security::validate_timeout_minutes(timeout_minutes)?;
        let settings = update_settings(&state, |security| {
            security.auto_lock_timeout_minutes = timeout_minutes;
        })
        .await?;
        state.lock_runtime().note_activity();

        let state_after = publish(&app, &state, &settings)?;
        record_session_audit(
            &state,
            actions::SESSION_AUTO_LOCK_UPDATE,
            serde_json::json!({ "timeout_minutes": timeout_minutes }),
        )
        .await;
        Ok(state_after)
    })
    .await
}

/// Accept or stop accepting a biometric at the lock screen. The PIN keeps
/// working either way.
#[tauri::command]
#[specta::specta]
pub async fn set_biometric_unlock(
    app: AppHandle,
    state: State<'_, DesktopState>,
    enabled: bool,
) -> Result<LockState, String> {
    run("set_biometric_unlock", async {
        if enabled && !settings(&state).await?.pin_set() {
            return Err(CommandError::Msg(
                "Set a PIN before turning on biometric unlock".to_string(),
            ));
        }
        let settings = update_settings(&state, |security| {
            security.biometric_unlock_enabled = enabled;
        })
        .await?;

        let state_after = publish(&app, &state, &settings)?;
        record_session_audit(
            &state,
            actions::SESSION_AUTO_LOCK_UPDATE,
            serde_json::json!({ "biometric_unlock_enabled": enabled }),
        )
        .await;
        Ok(state_after)
    })
    .await
}

// ---------------------------------------------------------------------------
// Startup and the idle watcher
// ---------------------------------------------------------------------------

/// Start locked when auto-lock is configured, before any window can render.
///
/// Runs synchronously in Tauri's `setup` so the flag is already set by the
/// time the webview's first `get_lock_state` arrives. The receipt is filed on
/// a spawned task because nothing may wait on S3 here.
pub(crate) fn lock_at_startup(app: &AppHandle) {
    if !config::has_config() {
        return;
    }
    let armed = match config::load_config() {
        Ok(cfg) => cfg.security.armed(),
        Err(error) => {
            tracing::warn!(error = %error, "could not read auto-lock settings at startup");
            false
        }
    };
    if !armed {
        return;
    }

    app.state::<DesktopState>().lock_runtime().lock();

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<DesktopState>();
        record_session_audit(
            &state,
            actions::SESSION_LOCK,
            serde_json::json!({ "reason": LockReason::Startup.as_str() }),
        )
        .await;
    });
}

/// Watch the idle timer and the wall clock.
///
/// Idle is measured from the frontend's throttled activity reports. Sleep is
/// inferred from the wall clock rather than a monotonic one: monotonic clocks
/// disagree about whether suspended time counts, and platforms disagree with
/// each other, while a wall-clock gap past one tick means the same thing
/// everywhere.
pub(crate) fn spawn_idle_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut previous = jiff::Timestamp::now();
        loop {
            tokio::time::sleep(WATCHER_TICK).await;

            let now = jiff::Timestamp::now();
            let slept = security::wall_clock_jumped(previous, now, WATCHER_TICK, SLEEP_SLACK);
            previous = now;

            let state = app.state::<DesktopState>();
            let settings = match settings(&state).await {
                Ok(settings) => settings,
                Err(error) => {
                    tracing::warn!(error = %error, "auto-lock watcher could not read settings");
                    continue;
                }
            };
            if !settings.armed() {
                continue;
            }

            let idle = {
                let timeout = security::idle_timeout(settings.auto_lock_timeout_minutes);
                state.lock_runtime().idle_expired(timeout)
            };
            let reason = if slept {
                LockReason::Sleep
            } else if idle {
                LockReason::Idle
            } else {
                continue;
            };

            if let Err(error) = engage_lock(&app, reason).await {
                tracing::warn!(error = %error, reason = reason.as_str(), "auto-lock failed");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// The biometric plugin
// ---------------------------------------------------------------------------

/// Ask the platform whether a biometric is usable right now.
///
/// The plugin returns an error rather than an empty status on platforms it
/// does not support (Linux has a stub that always errors), which is reported
/// here as "unavailable" — a machine with no sensor must not make the
/// Preferences pane fail to load.
async fn biometry_status(app: AppHandle) -> Result<BiometryAvailability, CommandError> {
    use tauri_plugin_biometry::BiometryExt;

    let status = tokio::task::spawn_blocking(move || app.biometry().status())
        .await
        .map_err(|error| CommandError::Msg(format!("biometry check did not complete: {error}")))?;

    Ok(match status {
        Ok(status) => BiometryAvailability {
            available: status.is_available,
            kind: biometry_kind(&status.biometry_type),
            error: status.error,
        },
        Err(error) => BiometryAvailability {
            available: false,
            kind: BiometryKind::None,
            error: Some(error.to_string()),
        },
    })
}

/// Whether any Claria window currently has OS focus.
///
/// Asked of Tauri rather than of the webview's DOM: `document.hasFocus()`
/// describes focus *within* the page and stays true in cases the window
/// manager does not consider the app frontmost. `is_focused()` is implemented
/// on every desktop platform Tauri supports, so this reads the same on macOS
/// and on Windows.
///
/// Any window counts. The lock overlay is raised in all of them, so unlocking
/// from the console window is as legitimate as unlocking from the main one.
/// A window that cannot answer is treated as unfocused — the failure mode
/// worth having is a press that does nothing, not a panel nobody asked for.
fn app_is_frontmost(app: &AppHandle) -> bool {
    app.webview_windows()
        .values()
        .any(|window| window.is_focused().unwrap_or(false))
}

/// Present the platform's biometric prompt.
///
/// The plugin's macOS backend blocks on a channel while the system panel is
/// up, so this has to be a blocking task. The inner `Result` is the prompt's
/// own answer — a rejection is not a failure of the command.
async fn prompt_for_biometric(
    app: AppHandle,
) -> Result<Result<(), tauri_plugin_biometry::Error>, CommandError> {
    use tauri_plugin_biometry::BiometryExt;

    tokio::task::spawn_blocking(move || {
        app.biometry().authenticate(
            "Unlock Claria".to_string(),
            tauri_plugin_biometry::AuthOptions {
                cancel_title: Some("Use PIN".to_string()),
                ..Default::default()
            },
        )
    })
    .await
    .map_err(|error| CommandError::Msg(format!("biometric prompt did not complete: {error}")))
}

fn biometry_kind(kind: &tauri_plugin_biometry::BiometryType) -> BiometryKind {
    match kind {
        tauri_plugin_biometry::BiometryType::TouchID => BiometryKind::TouchId,
        tauri_plugin_biometry::BiometryType::FaceID => BiometryKind::FaceId,
        // Windows reports `Auto` when Windows Hello is available; every other
        // platform that reports it means "some sensor, unnamed".
        #[cfg(target_os = "windows")]
        tauri_plugin_biometry::BiometryType::Auto => BiometryKind::WindowsHello,
        #[cfg(not(target_os = "windows"))]
        tauri_plugin_biometry::BiometryType::Auto => BiometryKind::Biometric,
        tauri_plugin_biometry::BiometryType::None => BiometryKind::None,
    }
}
