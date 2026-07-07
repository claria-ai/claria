import { useCallback, useEffect, useState } from "react";
import { events, getLockState, recordActivity, type LockState } from "../lib/tauri";
import LockScreen from "./LockScreen";

const ACTIVITY_THROTTLE_MS = 15_000;

/**
 * Wraps a window's content in the session-lock lifecycle: queries lock state
 * on mount (so a WebView reload can't bypass the lock), re-queries on the
 * lock-state-changed event, reports throttled user activity to the idle
 * timer, and renders the opaque LockScreen overlay while locked.
 */
export default function LockGate({ children }: { children: React.ReactNode }) {
  const [lockState, setLockState] = useState<LockState | null>(null);

  const refresh = useCallback(() => {
    const unlocked: LockState = {
      locked: false,
      auto_lock_enabled: false,
      biometric_unlock_enabled: false,
      pin_set: false,
      failed_attempts: 0,
      backoff_remaining_seconds: null,
    };
    // Coalesce a missing response (Playwright/screenshot mocks without
    // this command) so the app renders rather than a dead lock screen.
    getLockState()
      .then((s) => setLockState(s ?? unlocked))
      .catch(() => setLockState(unlocked));
  }, []);

  useEffect(() => {
    refresh();
    const unlisten = events.lockStateChanged.listen(() => {
      refresh();
    });
    return () => {
      unlisten.then((f) => f()).catch(() => {});
    };
  }, [refresh]);

  const armed = lockState !== null && lockState.auto_lock_enabled && !lockState.locked;
  useEffect(() => {
    if (!armed) return;
    let last = 0;
    const report = () => {
      const now = Date.now();
      if (now - last < ACTIVITY_THROTTLE_MS) return;
      last = now;
      recordActivity().catch(() => {});
    };
    const opts = { passive: true } as const;
    window.addEventListener("pointermove", report, opts);
    window.addEventListener("pointerdown", report, opts);
    window.addEventListener("keydown", report, opts);
    window.addEventListener("wheel", report, opts);
    report();
    return () => {
      window.removeEventListener("pointermove", report);
      window.removeEventListener("pointerdown", report);
      window.removeEventListener("keydown", report);
      window.removeEventListener("wheel", report);
    };
  }, [armed]);

  // Nothing renders until the first lock-state read resolves — otherwise a
  // reload would flash PHI before the overlay mounts.
  if (lockState === null) {
    return null;
  }

  return (
    <>
      {children}
      {lockState.locked && <LockScreen state={lockState} onRefresh={refresh} />}
    </>
  );
}
