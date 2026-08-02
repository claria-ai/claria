import { useEffect, useState } from "react";

// Distraction mode is a purely cosmetic per-machine toggle, so it lives in
// localStorage rather than the synced S3 preferences — enabling it on one
// computer says nothing about what a clinician wants on another.

const KEY = "claria.distraction_mode";

// Fallback for a locked-down webview with no persistent storage: the toggle
// still works for the lifetime of the app, it just doesn't survive a restart.
let inMemory = false;

const listeners = new Set<() => void>();

function notify() {
  listeners.forEach((listener) => listener());
}

export function isDistractionModeEnabled(): boolean {
  try {
    return window.localStorage.getItem(KEY) === "true";
  } catch {
    return inMemory;
  }
}

export function setDistractionModeEnabled(enabled: boolean): void {
  inMemory = enabled;
  try {
    window.localStorage.setItem(KEY, String(enabled));
  } catch {
    // Kept in memory only — see above.
  }
  notify();
}

/**
 * The toggle as React state, shared across every mounted subscriber — the
 * Preferences switch and the chat's sock button see each other's changes
 * without a page reload.
 */
export function useDistractionMode(): [boolean, (enabled: boolean) => void] {
  const [enabled, setEnabled] = useState(isDistractionModeEnabled);
  useEffect(() => {
    const listener = () => setEnabled(isDistractionModeEnabled());
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, []);
  return [enabled, setDistractionModeEnabled];
}
