import { useEffect, type ReactNode } from "react";
import { recordActivity } from "../lib/tauri";
import { logFrontendEvent } from "../lib/logBridge";
import { useLockState } from "../lib/useLockState";
import LockScreen from "./LockScreen";

/**
 * How rarely the frontend tells Rust the clinician is still here.
 *
 * This is a heartbeat, not an event stream: the idle timer is measured in
 * minutes, so one report per quarter-minute is plenty, and a pointer-move
 * handler crossing the IPC bridge on every frame would not be.
 */
const ACTIVITY_THROTTLE_MS = 15_000;

/**
 * Wraps a window in the session-lock lifecycle.
 *
 * Reads the lock on mount — so reloading the webview cannot step around the
 * overlay — follows the Rust-side broadcast, reports throttled activity into
 * the idle timer, and raises [`LockScreen`] over the children while locked.
 *
 * Nothing here decides to lock or unlock. It renders what the backend says.
 */
export default function LockGate({ children }: { children: ReactNode }) {
  const lockState = useLockState();

  const armed = lockState?.auto_lock_enabled === true && !lockState.locked;
  useEffect(() => {
    if (!armed) return;
    let last = 0;
    const report = () => {
      const now = Date.now();
      if (now - last < ACTIVITY_THROTTLE_MS) return;
      last = now;
      recordActivity().catch((reason) =>
        logFrontendEvent("warn", `activity report failed: ${reason}`)
      );
    };
    const options = { passive: true } as const;
    window.addEventListener("pointermove", report, options);
    window.addEventListener("pointerdown", report, options);
    window.addEventListener("keydown", report, options);
    window.addEventListener("wheel", report, options);
    // Arming mid-session starts the clock now, not from whenever the last
    // pointer event happened to be.
    report();
    return () => {
      window.removeEventListener("pointermove", report);
      window.removeEventListener("pointerdown", report);
      window.removeEventListener("keydown", report);
      window.removeEventListener("wheel", report);
    };
  }, [armed]);

  // Nothing renders until the first read lands. Showing the app first and the
  // overlay a tick later is a frame of PHI on an unattended screen.
  if (lockState === null) return null;

  return (
    <>
      {children}
      {lockState.locked && <LockScreen state={lockState} />}
    </>
  );
}
