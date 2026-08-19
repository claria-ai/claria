import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LockState } from "../lib/tauri";

vi.mock("../lib/logBridge", () => ({ logFrontendEvent: vi.fn() }));
vi.mock("../lib/tauri", () => ({
  unlockWithPin: vi.fn(),
  unlockWithBiometric: vi.fn(),
  getBiometryStatus: vi.fn(),
}));
vi.mock("../lib/windowFocus", () => ({ isWindowFocused: vi.fn() }));

import LockScreen from "./LockScreen";
import {
  getBiometryStatus,
  unlockWithBiometric,
  unlockWithPin,
} from "../lib/tauri";
import { isWindowFocused } from "../lib/windowFocus";

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

/** A lock screen with Touch ID offered, which is when the button exists. */
const WITH_TOUCH_ID: LockState = { ...LOCKED, biometric_unlock_enabled: true };

/** Renders with the sensor available and waits for the button to appear. */
async function renderWithSensor(): Promise<HTMLElement> {
  vi.mocked(getBiometryStatus).mockResolvedValue({
    available: true,
    kind: "touch_id",
    error: null,
  });
  render(<LockScreen state={WITH_TOUCH_ID} />);
  return await screen.findByRole("button", { name: "Unlock with Touch ID" });
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getBiometryStatus).mockResolvedValue({
    available: false,
    kind: "none",
    error: null,
  });
  vi.mocked(isWindowFocused).mockResolvedValue(true);
  vi.mocked(unlockWithBiometric).mockResolvedValue({
    accepted: false,
    state: LOCKED,
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

  /**
   * The regression this suite exists for.
   *
   * Auto-lock fires on an idle timer, so the overlay routinely appears while
   * the clinician is working in another application. The biometric panel is
   * system-modal and always-on-top: raising it without a press drops an OS
   * dialog over whatever they were actually doing.
   */
  it("never reaches for the sensor on its own", async () => {
    const button = await renderWithSensor();

    expect(vi.mocked(unlockWithBiometric)).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();

    // Settling every pending effect and promise, because "not yet" is not the
    // property under test — "not without a press" is.
    await act(async () => {
      await Promise.resolve();
    });
    expect(vi.mocked(unlockWithBiometric)).not.toHaveBeenCalled();

    // The button is the only way to it.
    await userEvent.setup().click(button);
    expect(vi.mocked(unlockWithBiometric)).toHaveBeenCalledTimes(1);
  });

  it("stays inert while Claria is not the frontmost app", async () => {
    vi.mocked(isWindowFocused).mockResolvedValue(false);
    const button = await renderWithSensor();

    // A click queued before the window went away, or a stray key on a focused
    // control, must not be able to raise a system panel over another app.
    await userEvent.setup().click(button);
    expect(vi.mocked(unlockWithBiometric)).not.toHaveBeenCalled();

    // Silently, too: an error box would be telling the clinician off in a
    // window they are not looking at.
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /**
   * The panel's cancel button is labelled "Use PIN", and pressing it arrives
   * as `userCancel`. Someone who just asked to type their PIN must not be met
   * with a red box — least of all one reading `[userCancel] - Authentication
   * canceled.`, which is what LocalAuthentication's own text says.
   */
  it("says nothing when the panel is dismissed, and hands back the PIN field", async () => {
    const button = await renderWithSensor();
    await userEvent.setup().click(button);

    await waitFor(() =>
      expect(vi.mocked(unlockWithBiometric)).toHaveBeenCalledTimes(1)
    );
    expect(screen.queryByRole("alert")).toBeNull();
    expect(document.body.textContent).not.toContain("userCancel");
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByLabelText("PIN"))
    );
  });

  it("shows a real biometric failure as a sentence", async () => {
    const button = await renderWithSensor();
    // Rust has already mapped the LocalAuthentication case to prose; the
    // overlay's job is only to render what it was handed.
    vi.mocked(unlockWithBiometric).mockResolvedValue({
      accepted: false,
      state: LOCKED,
      error: "Not recognised. Enter your PIN instead.",
    });

    await userEvent.setup().click(button);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Not recognised");
    expect(alert.textContent).not.toContain("[");
  });

  it("says nothing about biometrics when the setting is off", async () => {
    render(<LockScreen state={LOCKED} />);
    await screen.findByLabelText("PIN");
    expect(vi.mocked(getBiometryStatus)).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /Unlock with/ })).toBeNull();
  });
});
