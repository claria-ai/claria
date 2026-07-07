import { useEffect, useRef, useState } from "react";
import {
  getBiometryStatus,
  unlockWithBiometric,
  unlockWithPin,
  type LockState,
} from "../lib/tauri";

function biometricLabel(kind: string): string {
  switch (kind) {
    case "touch_id":
      return "Unlock with Touch ID";
    case "face_id":
      return "Unlock with Face ID";
    case "windows_hello":
      return "Unlock with Windows Hello";
    default:
      return "Unlock with biometrics";
  }
}

/**
 * Opaque full-screen overlay shown while the session is locked. Must fully
 * hide the page behind it — the React tree underneath stays mounted so
 * unsaved drafts survive the lock.
 */
export default function LockScreen({
  state,
  onRefresh,
}: {
  state: LockState;
  onRefresh: () => void;
}) {
  const [pin, setPin] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [bioKind, setBioKind] = useState<string | null>(null);
  const [backoff, setBackoff] = useState(state.backoff_remaining_seconds ?? 0);
  const bioAutoTried = useRef(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setBackoff(state.backoff_remaining_seconds ?? 0);
  }, [state.backoff_remaining_seconds]);

  const inBackoff = backoff > 0;
  useEffect(() => {
    if (!inBackoff) return;
    const t = setInterval(() => setBackoff((s) => Math.max(0, s - 1)), 1000);
    return () => clearInterval(t);
  }, [inBackoff]);

  useEffect(() => {
    inputRef.current?.focus();
  }, [inBackoff]);

  async function tryBiometric() {
    setBusy(true);
    setError(null);
    try {
      await unlockWithBiometric();
      // Success unlocks via the lock-state-changed event; nothing to do here.
    } catch (e) {
      const msg = String(e);
      // A cancelled prompt is not an error worth shouting about.
      if (!msg.includes("userCancel")) {
        setError(msg);
      }
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (!state.biometric_unlock_enabled) return;
    getBiometryStatus()
      .then((s) => {
        if (!s.available) return;
        setBioKind(s.kind);
        // Prompt once automatically when the lock screen appears; after a
        // cancel the user retries via the button.
        if (!bioAutoTried.current) {
          bioAutoTried.current = true;
          tryBiometric();
        }
      })
      .catch(() => {});
  }, [state.biometric_unlock_enabled]);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (busy || inBackoff || pin.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      await unlockWithPin(pin);
    } catch (err) {
      setError(String(err));
      setPin("");
      onRefresh();
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 bg-gray-100 flex items-center justify-center">
      <div className="bg-white border border-gray-200 rounded-xl shadow-sm p-8 w-80 text-center">
        <div className="text-3xl mb-2" aria-hidden>
          &#128274;
        </div>
        <h1 className="text-lg font-semibold text-gray-900">Claria is locked</h1>
        <p className="text-xs text-gray-400 mt-1 mb-5">
          Enter your PIN to continue
        </p>

        <form onSubmit={handleSubmit}>
          <input
            ref={inputRef}
            type="password"
            inputMode="numeric"
            autoComplete="off"
            value={pin}
            onChange={(e) => setPin(e.target.value.replace(/\D/g, "").slice(0, 12))}
            disabled={busy || inBackoff}
            placeholder={inBackoff ? `Try again in ${backoff} s` : "PIN"}
            className="w-full border border-gray-300 rounded-lg px-3 py-2 text-center tracking-[0.3em] text-lg disabled:bg-gray-50 disabled:text-gray-400"
            aria-label="PIN"
          />
          <button
            type="submit"
            disabled={busy || inBackoff || pin.length === 0}
            className="w-full mt-3 bg-gray-900 text-white rounded-lg py-2 text-sm font-medium disabled:opacity-40"
          >
            Unlock
          </button>
        </form>

        {state.biometric_unlock_enabled && bioKind && (
          <button
            type="button"
            onClick={tryBiometric}
            disabled={busy}
            className="w-full mt-2 border border-gray-300 text-gray-700 rounded-lg py-2 text-sm disabled:opacity-40"
          >
            {biometricLabel(bioKind)}
          </button>
        )}

        {error && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-2 mt-3">
            <p className="text-red-800 text-xs">{error}</p>
          </div>
        )}
      </div>
    </div>
  );
}
