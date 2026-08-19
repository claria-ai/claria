import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getBiometryStatus,
  unlockWithBiometric,
  unlockWithPin,
  type LockState,
} from "../lib/tauri";
import { biometricLabel } from "../lib/biometry";
import { logFrontendEvent } from "../lib/logBridge";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import { LockIcon } from "./icons";

/** Matches `MAX_PIN_LEN` in `claria-desktop/src/security.rs`. */
const MAX_PIN_LENGTH = 12;

/** How often the backoff countdown redraws. */
const COUNTDOWN_TICK_MS = 500;

/**
 * The opaque overlay shown while the session is locked.
 *
 * It covers the page rather than replacing it: the React tree underneath stays
 * mounted, so an unsaved draft survives a lock. That makes opacity the whole
 * security property — anything that lets colour through here shows PHI to
 * whoever walked past the desk.
 *
 * Rust decides whether the credential was right. This renders the answer and
 * counts down the wait it earned.
 */
export default function LockScreen({ state }: { state: LockState }) {
  const [pin, setPin] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const promptedOnce = useRef(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Hold the deadline the backoff ends at, not the seconds left. Each state
  // broadcast is a fresh object, so a new refusal restarts the countdown while
  // an unrelated re-render cannot.
  const deadline = useMemo(
    () =>
      state.backoff_remaining_seconds
        ? Date.now() + state.backoff_remaining_seconds * 1000
        : 0,
    [state]
  );
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (deadline <= Date.now()) return;
    const timer = setInterval(() => setNow(Date.now()), COUNTDOWN_TICK_MS);
    return () => clearInterval(timer);
  }, [deadline]);

  const remaining = Math.max(0, Math.ceil((deadline - now) / 1000));
  const waiting = remaining > 0;

  useEffect(() => {
    if (!waiting) inputRef.current?.focus();
  }, [waiting]);

  const { data: biometry, error: biometryError } = useAsyncLoad(
    state.biometric_unlock_enabled ? getBiometryStatus : null,
    [state.biometric_unlock_enabled]
  );
  useEffect(() => {
    if (biometryError) {
      logFrontendEvent("warn", `biometry unavailable: ${biometryError}`);
    }
  }, [biometryError]);

  const tryBiometric = useCallback(async () => {
    setBusy(true);
    setMessage(null);
    try {
      const outcome = await unlockWithBiometric();
      // A dismissed prompt comes back with no error — the clinician reached
      // for the PIN field, which is a choice and not a failure.
      if (!outcome.accepted && outcome.error) setMessage(outcome.error);
    } catch (reason) {
      logFrontendEvent("warn", `biometric unlock failed: ${reason}`);
      setMessage(String(reason));
    } finally {
      setBusy(false);
    }
  }, []);

  // Present the sensor once when the overlay appears; after that the button
  // is the way back to it.
  useEffect(() => {
    if (!biometry?.available || promptedOnce.current) return;
    promptedOnce.current = true;
    tryBiometric();
  }, [biometry, tryBiometric]);

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    if (busy || waiting || pin.length === 0) return;
    setBusy(true);
    setMessage(null);
    try {
      const outcome = await unlockWithPin(pin);
      if (!outcome.accepted) {
        setPin("");
        const wait = outcome.state.backoff_remaining_seconds;
        setMessage(
          outcome.error ??
            (wait ? `Incorrect PIN. Try again in ${wait} s.` : "Incorrect PIN.")
        );
      }
    } catch (reason) {
      logFrontendEvent("error", `PIN unlock failed: ${reason}`);
      setMessage(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-gray-100"
      data-testid="lock-screen"
    >
      <div className="w-80 rounded-xl border border-gray-200 bg-white p-8 text-center shadow-sm">
        <LockIcon className="mx-auto mb-3 h-7 w-7 text-gray-400" />
        <h1 className="text-lg font-semibold text-gray-900">
          Claria is locked
        </h1>
        <p className="mt-1 mb-5 text-xs text-gray-400">
          Enter your PIN to continue.
        </p>

        <form onSubmit={handleSubmit}>
          <input
            ref={inputRef}
            type="password"
            inputMode="numeric"
            autoComplete="off"
            value={pin}
            onChange={(event) =>
              setPin(
                event.target.value.replace(/\D/g, "").slice(0, MAX_PIN_LENGTH)
              )
            }
            disabled={busy || waiting}
            placeholder={waiting ? `Try again in ${remaining} s` : "PIN"}
            className="w-full rounded-lg border border-gray-300 px-3 py-2 text-center text-lg tracking-[0.3em] disabled:bg-gray-50 disabled:text-gray-400"
            aria-label="PIN"
          />
          <button
            type="submit"
            disabled={busy || waiting || pin.length === 0}
            className="mt-3 w-full rounded-lg bg-gray-900 py-2 text-sm font-medium text-white disabled:opacity-40"
          >
            Unlock
          </button>
        </form>

        {biometry?.available && (
          <button
            type="button"
            onClick={tryBiometric}
            disabled={busy}
            className="mt-2 w-full rounded-lg border border-gray-300 py-2 text-sm text-gray-700 disabled:opacity-40"
          >
            {biometricLabel(biometry.kind)}
          </button>
        )}

        {message && (
          <div
            className="mt-3 rounded-lg border border-red-200 bg-red-50 p-2"
            role="alert"
          >
            <p className="text-xs text-red-800">{message}</p>
          </div>
        )}
      </div>
    </div>
  );
}
