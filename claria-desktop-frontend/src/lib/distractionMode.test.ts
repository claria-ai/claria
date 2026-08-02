import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import {
  isDistractionModeEnabled,
  setDistractionModeEnabled,
  useDistractionMode,
} from "./distractionMode";

beforeEach(() => {
  window.localStorage.clear();
});

describe("distraction mode preference", () => {
  it("defaults to off", () => {
    expect(isDistractionModeEnabled()).toBe(false);
  });

  it("round-trips through localStorage", () => {
    setDistractionModeEnabled(true);
    expect(isDistractionModeEnabled()).toBe(true);
    expect(window.localStorage.getItem("claria.distraction_mode")).toBe("true");

    setDistractionModeEnabled(false);
    expect(isDistractionModeEnabled()).toBe(false);
  });

  it("treats junk stored values as off", () => {
    window.localStorage.setItem("claria.distraction_mode", "banana");
    expect(isDistractionModeEnabled()).toBe(false);
  });

  it("keeps every mounted hook in sync", () => {
    const first = renderHook(() => useDistractionMode());
    const second = renderHook(() => useDistractionMode());
    expect(first.result.current[0]).toBe(false);
    expect(second.result.current[0]).toBe(false);

    // Toggling through one hook updates the other — this is what lets the
    // chat's sock button appear as soon as the Preferences switch flips.
    act(() => first.result.current[1](true));
    expect(first.result.current[0]).toBe(true);
    expect(second.result.current[0]).toBe(true);

    act(() => second.result.current[1](false));
    expect(first.result.current[0]).toBe(false);
  });
});
