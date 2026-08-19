import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LockState } from "../lib/tauri";

vi.mock("../lib/logBridge", () => ({ logFrontendEvent: vi.fn() }));
vi.mock("../lib/tauri", () => ({
  unlockWithPin: vi.fn(),
  unlockWithBiometric: vi.fn(),
  getBiometryStatus: vi.fn(),
}));

import LockScreen from "./LockScreen";
import {
  getBiometryStatus,
  unlockWithBiometric,
  unlockWithPin,
} from "../lib/tauri";

const LOCKED: LockState = {
  locked: true,
  auto_lock_enabled: true,
  auto_lock_timeout_minutes: 5,
  biometric_unlock_enabled: false,
  pin_set: true,
  failed_attempts: 0,
  backoff_remaining_seconds: null,
};

function rejected(backoffSeconds: number | null): {
  accepted: false;
  state: LockState;
  error: null;
} {
  return {
    accepted: false,
    state: { ...LOCKED, backoff_remaining_seconds: backoffSeconds },
    error: null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getBiometryStatus).mockResolvedValue({
    available: false,
    kind: "none",
    error: null,
  });
});

describe("LockScreen", () => {
  it("covers the whole viewport with an opaque layer", () => {
    render(<LockScreen state={LOCKED} />);
    const overlay = screen.getByTestId("lock-screen");
    // The tree underneath stays mounted, so occlusion is the whole property:
    // a transparent or inset overlay leaks PHI onto an unattended screen.
    expect(overlay.className).toContain("fixed");
    expect(overlay.className).toContain("inset-0");
    expect(overlay.className).toMatch(/\bbg-gray-100\b/);
  });

  it("keeps the PIN out of the DOM as readable text", async () => {
    const user = userEvent.setup();
    render(<LockScreen state={LOCKED} />);
    const field = screen.getByLabelText("PIN");
    await user.type(field, "482913");
    expect((field as HTMLInputElement).type).toBe("password");
    expect(document.body.textContent).not.toContain("482913");
  });

  it("drops non-digits and stops at twelve characters", async () => {
    const user = userEvent.setup();
    render(<LockScreen state={LOCKED} />);
    const field = screen.getByLabelText("PIN") as HTMLInputElement;
    await user.type(field, "12ab34-5678901234");
    expect(field.value).toBe("123456789012");
  });

  it("reports a rejected PIN in its own words", async () => {
    const user = userEvent.setup();
    vi.mocked(unlockWithPin).mockResolvedValue(rejected(null));

    render(<LockScreen state={LOCKED} />);
    await user.type(screen.getByLabelText("PIN"), "999999");
    await user.click(screen.getByRole("button", { name: "Unlock" }));

    // Not a thrown command error: a mistyped PIN must never reach the reader
    // as a command name and a flattened error chain.
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Incorrect PIN");
    expect((screen.getByLabelText("PIN") as HTMLInputElement).value).toBe("");
  });

  it("refuses further attempts once Rust broadcasts a backoff", async () => {
    const user = userEvent.setup();
    const outcome = rejected(20);
    vi.mocked(unlockWithPin).mockResolvedValue(outcome);

    const { rerender } = render(<LockScreen state={LOCKED} />);
    await user.type(screen.getByLabelText("PIN"), "999999");
    await user.click(screen.getByRole("button", { name: "Unlock" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Try again in 20 s");

    // Rust publishes the refusal to every window; the gate hands it back down.
    rerender(<LockScreen state={outcome.state} />);
    expect((screen.getByLabelText("PIN") as HTMLInputElement).disabled).toBe(
      true
    );
    expect(
      (screen.getByRole("button", { name: "Unlock" }) as HTMLButtonElement)
        .disabled
    ).toBe(true);
  });

  it("starts in backoff when the lock state arrives already waiting", () => {
    render(
      <LockScreen state={{ ...LOCKED, backoff_remaining_seconds: 45 }} />
    );
    const field = screen.getByLabelText("PIN") as HTMLInputElement;
    expect(field.disabled).toBe(true);
    expect(field.placeholder).toBe("Try again in 45 s");
  });

  it("prompts the sensor once and offers a button back to it", async () => {
    vi.mocked(getBiometryStatus).mockResolvedValue({
      available: true,
      kind: "touch_id",
      error: null,
    });
    vi.mocked(unlockWithBiometric).mockResolvedValue({
      accepted: false,
      state: LOCKED,
      error: null,
    });

    render(
      <LockScreen state={{ ...LOCKED, biometric_unlock_enabled: true }} />
    );

    const retry = await screen.findByRole("button", {
      name: "Unlock with Touch ID",
    });
    expect(vi.mocked(unlockWithBiometric)).toHaveBeenCalledTimes(1);

    // A dismissed prompt says nothing — the clinician reached for the PIN.
    expect(screen.queryByRole("alert")).toBeNull();

    await userEvent.setup().click(retry);
    expect(vi.mocked(unlockWithBiometric)).toHaveBeenCalledTimes(2);
  });

  it("says nothing about biometrics when the setting is off", async () => {
    render(<LockScreen state={LOCKED} />);
    await screen.findByLabelText("PIN");
    expect(vi.mocked(getBiometryStatus)).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /Unlock with/ })).toBeNull();
  });
});
