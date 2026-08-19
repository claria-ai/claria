import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LockState } from "../lib/tauri";

vi.mock("../lib/logBridge", () => ({ logFrontendEvent: vi.fn() }));

type Listener = (event: { payload: { state: LockState } }) => void;
const listeners: Listener[] = [];

vi.mock("../lib/tauri", () => ({
  getLockState: vi.fn(),
  recordActivity: vi.fn(async () => {}),
  unlockWithPin: vi.fn(),
  unlockWithBiometric: vi.fn(),
  getBiometryStatus: vi.fn(async () => ({
    available: false,
    kind: "none",
    error: null,
  })),
  events: {
    lockStateChanged: {
      listen: vi.fn(async (listener: Listener) => {
        listeners.push(listener);
        return () => {
          listeners.splice(listeners.indexOf(listener), 1);
        };
      }),
    },
  },
}));

import LockGate from "./LockGate";
import { getLockState, recordActivity } from "../lib/tauri";
import { logFrontendEvent } from "../lib/logBridge";

const UNLOCKED: LockState = {
  locked: false,
  auto_lock_enabled: false,
  auto_lock_timeout_minutes: 5,
  biometric_unlock_enabled: false,
  pin_set: false,
  failed_attempts: 0,
  backoff_remaining_seconds: null,
};

function broadcast(state: LockState) {
  act(() => {
    for (const listener of [...listeners]) listener({ payload: { state } });
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  listeners.length = 0;
  vi.mocked(getLockState).mockResolvedValue(UNLOCKED);
});

describe("LockGate", () => {
  it("renders nothing until the first lock read lands", async () => {
    let resolve: ((state: LockState) => void) | undefined;
    vi.mocked(getLockState).mockReturnValue(
      new Promise<LockState>((r) => {
        resolve = r;
      })
    );

    render(
      <LockGate>
        <p>patient notes</p>
      </LockGate>
    );

    // A frame of records on screen before the overlay mounts is the exact
    // thing this feature exists to prevent.
    expect(screen.queryByText("patient notes")).toBeNull();

    await act(async () => resolve?.(UNLOCKED));
    expect(screen.getByText("patient notes")).toBeTruthy();
  });

  it("raises the overlay when Rust says the session locked", async () => {
    render(
      <LockGate>
        <p>patient notes</p>
      </LockGate>
    );
    await screen.findByText("patient notes");
    expect(screen.queryByTestId("lock-screen")).toBeNull();

    broadcast({ ...UNLOCKED, locked: true, auto_lock_enabled: true, pin_set: true });

    expect(screen.getByTestId("lock-screen")).toBeTruthy();
    // The tree underneath stays mounted, so an unsaved draft survives a lock.
    expect(screen.getByText("patient notes")).toBeTruthy();
  });

  it("renders the app when the lock command is not there to answer", async () => {
    vi.mocked(getLockState).mockRejectedValue("unknown command");

    render(
      <LockGate>
        <p>patient notes</p>
      </LockGate>
    );

    // Degrading is allowed; doing it silently is not.
    await screen.findByText("patient notes");
    expect(vi.mocked(logFrontendEvent)).toHaveBeenCalled();
  });

  it("only feeds the idle timer once auto-lock is armed", async () => {
    render(
      <LockGate>
        <p>patient notes</p>
      </LockGate>
    );
    await screen.findByText("patient notes");
    expect(vi.mocked(recordActivity)).not.toHaveBeenCalled();

    broadcast({ ...UNLOCKED, auto_lock_enabled: true, pin_set: true });

    await waitFor(() =>
      expect(vi.mocked(recordActivity)).toHaveBeenCalledTimes(1)
    );

    // Throttled hard: the timer is measured in minutes, so a burst of pointer
    // events must not become a burst of IPC calls.
    act(() => {
      for (let i = 0; i < 10; i += 1) {
        window.dispatchEvent(new Event("pointermove"));
      }
    });
    expect(vi.mocked(recordActivity)).toHaveBeenCalledTimes(1);
  });

  it("stops feeding the idle timer while locked", async () => {
    render(
      <LockGate>
        <p>patient notes</p>
      </LockGate>
    );
    await screen.findByText("patient notes");

    broadcast({
      ...UNLOCKED,
      locked: true,
      auto_lock_enabled: true,
      pin_set: true,
    });
    act(() => window.dispatchEvent(new Event("keydown")));

    expect(vi.mocked(recordActivity)).not.toHaveBeenCalled();
  });
});
