import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  isDistractionModeEnabled,
  setDistractionModeEnabled,
  useDistractionMode,
} from "./distractionMode";

beforeEach(() => {
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
    clear: () => stored.clear(),
  });
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
    // header sock button appear as soon as the Preferences switch flips.
    act(() => first.result.current[1](true));
    expect(first.result.current[0]).toBe(true);
    expect(second.result.current[0]).toBe(true);

    act(() => second.result.current[1](false));
    expect(first.result.current[0]).toBe(false);
  });
});
