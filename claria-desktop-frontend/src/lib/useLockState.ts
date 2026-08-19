import { useEffect, useState } from "react";
import { events, getLockState, type LockState } from "./tauri";
import { logFrontendEvent } from "./logBridge";
import { useAsyncLoad } from "./useAsyncLoad";

/**
 * What a window sees when there is no lock backend behind it — a screenshot
 * harness, a Playwright mock, a demo recording. Rendering the app is the only
 * safe answer there; an overlay nothing can dismiss is not.
 */
const UNLOCKED: LockState = {
  locked: false,
  auto_lock_enabled: false,
  auto_lock_timeout_minutes: 5,
  biometric_unlock_enabled: false,
  pin_set: false,
  failed_attempts: 0,
  backoff_remaining_seconds: null,
};

/**
 * The session lock as React state: one read on mount, then whatever Rust
 * broadcasts.
 *
 * Rust owns the lock. Every command that changes it publishes the result to
 * every window, so a caller never refetches after a mutation and the console
 * window sees a lock raised from the main one. `null` means the first read has
 * not landed yet — callers must render nothing rather than guess.
 */
export function useLockState(): LockState | null {
  const { data } = useAsyncLoad(
    () =>
      getLockState().catch((reason) => {
        logFrontendEvent("warn", `lock state unavailable: ${reason}`);
        return UNLOCKED;
      }),
    []
  );
  const [broadcast, setBroadcast] = useState<LockState | null>(null);

  useEffect(() => {
    const unlisten = events.lockStateChanged.listen((event) =>
      setBroadcast(event.payload.state)
    );
    return () => {
      unlisten
        .then((stop) => stop())
        .catch((reason) =>
          logFrontendEvent("warn", `lock listener teardown failed: ${reason}`)
        );
    };
  }, []);

  // A broadcast is always newer than the mount read, whichever resolves first.
  return broadcast ?? data;
}
