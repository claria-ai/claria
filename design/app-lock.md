# The session lock

Opt-in walk-away protection: after an idle period, on wake from sleep, at
startup, or on demand, Claria hides itself behind a PIN.

## What it defends against, and what it does not

The threat is a clinician's office with the door open and the screen still on
— someone glancing at a monitor, or sitting down at an unattended desk and
clicking around. Against that, an opaque overlay the app refuses to remove
without a credential is the whole defence, and it is enough.

It is not a defence against a local attacker with the machine and a debugger.
Commands are **not** gated while locked: a devtools console or a crafted IPC
call still reaches every command. Closing that gap means moving the check into
the command boundary itself, which is a different feature with a different
cost, and pretending otherwise here would be worse than saying so.

Nor is it disk encryption. `config.json`, the S3 credentials it holds, and
everything in the bucket are exactly as reachable while locked as while
unlocked. FileVault and the OS login remain the layer that protects a powered-
down machine.

## Where the state lives

| Piece | Where |
|---|---|
| Whether the session is locked, the idle clock, the failure counter | `claria-desktop/src/security.rs` — `LockRuntime`, held in `DesktopState` |
| PIN hash, idle timeout, biometric toggle | `claria-desktop/src/config.rs` — `SecuritySettings`, config v13 |
| Commands, the watcher, and the audit calls | `claria-desktop/src/commands/security.rs` |
| The overlay and the idle heartbeat | `components/LockGate.tsx`, `components/LockScreen.tsx`, `lib/useLockState.ts` |
| The settings pane | Preferences → Security, registered in `lib/preferencesNav.ts` |

`LockRuntime` is process-local and deliberately never persisted. A launch
derives its lock state from the config alone, so nothing on disk can leave the
app stuck locked, and the failure counter starts fresh on every launch.

The stored settings are machine-local by construction: `SecuritySettings` is
not part of `SyncedPreferences`, so it never reaches `_state/preferences.json`.
A PIN hash in the bucket would be a credential sitting inside the thing it
protects, and a lock set on the office desktop would follow the clinician onto
their laptop uninvited.

## The PIN

Six to twelve ASCII digits, stored as an argon2id PHC string. The parameters
are inside the hash, so raising them later does not invalidate what is already
stored.

Three constraints hold everywhere the PIN appears:

1. **Never in plaintext at rest.** Only the hash is written.
2. **Never in a log.** The hash is wrapped in `claria_core::sensitive::Sensitive`,
   so `{:?}` on a whole `ClariaConfig` renders `[redacted]` rather than the
   hash. That is not decoration — a config debug-print is one error path away
   from a support export.
3. **Never over IPC.** `LockState` — the only lock type the frontend sees —
   has no field that could hold it. It reports `pin_set: bool`.

Hashing and verification run on a blocking thread. argon2 is slow on purpose,
which is exactly why it cannot run on an async worker.

There is no recovery path. A forgotten PIN means clearing Claria's local
settings on that computer; nothing in S3 is affected.

## Backoff

Three free attempts, then five seconds, doubling per further failure, capped at
five minutes. The window is checked *before* the argon2 verification, so a
flood of attempts cannot use the verification itself as the workload.

The counter is in memory, so restarting the app clears it. The app restarts
locked when auto-lock is on, so a restart loop buys attempts rather than
access; persisting the counter would close that and is the obvious next
hardening step.

## A rejected credential is not an error

`unlock_with_pin`, `disable_auto_lock`, and `change_pin` return `LockOutcome`
with `accepted: false` rather than `Err`. The command layer prefixes every
flattened error with its operation name, so routing a mistyped PIN through the
error channel would show a clinician `unlock_with_pin: Incorrect PIN`. The
outcome type lets the lock screen phrase it, and carries the fresh `LockState`
so the caller learns the backoff it just earned in the same round trip.

`Err` is reserved for what it should be: no PIN configured, a stored hash that
will not parse, a config that will not save.

## Locking, and what triggers it

A watcher ticks every ten seconds and asks two questions.

**Idle.** The frontend reports activity — pointer, key, wheel — throttled to
one call per fifteen seconds. The timer is measured in minutes, so that is a
heartbeat rather than an event stream, and a pointer handler crossing the IPC
bridge on every frame would not be. Activity while locked is ignored, so
mashing keys at the lock screen cannot feed the timer that decides re-locking.

**Sleep.** A wall-clock gap past one tick plus a minute means the machine was
suspended. The wall clock rather than a monotonic one: monotonic clocks
disagree across platforms about whether suspended time counts, and a
wall-clock gap means the same thing everywhere.

The consequence is that the app locks on *wake*, not at sleep onset, and an OS
screen lock without sleep is covered only by the idle timer. Native hooks
(`com.apple.screenIsLocked`, `WTSRegisterSessionNotification`) would close
that; they are not wired up.

Startup locks synchronously inside Tauri's `setup`, before any window exists,
so the flag is already set when the webview's first `get_lock_state` arrives.

Locking is refused when no PIN is stored. An overlay with no credential that
dismisses it is not protection; it is a clinician locked out of their records.

## Broadcast, not polling

Every command that changes the lock publishes a `LockStateChanged` event
carrying the whole new state. `useLockState` reads once on mount and then
follows the broadcast, which is why no caller refetches after a mutation and
why a lock raised from the main window reaches the console window.

The mount read is what makes reloading the webview useless as a bypass: the
overlay is re-derived from Rust, not from anything the page held.

`LockGate` renders nothing at all until that first read lands. Showing the app
first and the overlay a tick later is a frame of records on an unattended
screen.

## The overlay

`LockScreen` covers the page rather than replacing it, so the React tree
underneath stays mounted and an unsaved draft survives a lock. That makes
opacity the entire security property of the component: anything that lets
colour through, or leaves a gap, defeats it. The e2e test asserts occlusion by
sampling `elementFromPoint`, not by asking whether content is "visible".

## Biometrics

`tauri-plugin-biometry` 0.2.8, driven entirely from Rust — no JS plugin
package, no capability entries, because the frontend never invokes the plugin
directly. macOS goes through `LAContext`, Windows through
`UserConsentVerifier`. The plugin's prompt call blocks on a channel while the
system panel is up, so it runs on a blocking thread.

Biometric unlock cannot be enabled without a PIN: it is a convenience over the
credential, never a replacement for it.

**The sensor is invoked only by a press on the lock screen, and only while
Claria is frontmost.** Both halves matter because the panel is system-modal and
always-on-top and auto-lock fires on an idle timer: anything that raised it
without a press would put an OS dialog over whatever application the clinician
had walked over to. The frontend checks focus before calling, and
`unlock_with_biometric` checks again immediately before the panel goes up,
because the window can lose focus in the gap. Both read Tauri's `is_focused()`
rather than `document.hasFocus()` — the DOM reports focus *within* the page and
stays true when the window manager does not consider the app frontmost — and
`is_focused()` is the same call on macOS and Windows. A call that arrives
unfocused returns a dismissal and says nothing.

A dismissed prompt is not a failure: it does not count against the backoff
budget, it is not audited as `session.unlock_failed`, and it renders no error.
The panel's cancel button is Claria's own `cancel_title`, "Use PIN", so
pressing it is someone asking for the PIN field — and it arrives as
`userCancel`, alongside `userFallback`, `appCancel`, and `systemCancel`.

Every other outcome goes through `security::biometric_failure_message`, which
maps the plugin's code to a sentence. No `LAError` case name reaches a
clinician; unrecognised codes fall to a generic sentence rather than through,
and the raw text is logged instead. The mapping reads the code out of the
error's `Display` text because the plugin re-exports only `Error` and `Result`
— its `ErrorResponse`, which holds the code as a field, is private. The macOS
and Windows backends use the same code names, so one mapping serves both.

Linux has no backend; the plugin's stub errors, which is reported as
"unavailable" so the toggle simply does not appear. PIN unlock is unaffected.

## The audit trail

Seven `session.*` actions, all `EventCategory::Admin`: locking and unlocking
are operational events over PHI, not reads of it. They go through
`claria_desktop::audit::record`, so they reach the durable S3 trail and not
only the console.

`details` carries a closed vocabulary — the lock reason (`idle`, `sleep`,
`startup`, `manual`), the unlock method (`pin`, `biometric`), an attempt count,
a backoff duration, a timeout in minutes. Nothing derived from the PIN appears
in any of them; `details` is console-safe by contract and these are the events
most likely to be exported after an incident.

The event is published to the windows *before* the audit write in every path,
so the overlay goes up whether or not S3 answers.
