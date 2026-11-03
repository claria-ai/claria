import { createContext } from "react";

/**
 * Whether the session lock is covering the app. `LockGate` provides it; outside
 * a gate (tests, the local demo) nothing is locked.
 */
export const SessionLockedContext = createContext(false);
