import { act, cleanup, fireEvent, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LuciaBehaviorConfig } from "./luciaBehavior";
import { useLuciaBehavior } from "./useLuciaBehavior";
import { getLuciaInferenceCount, withLuciaInference } from "./luciaInference";

const config: LuciaBehaviorConfig = {
  durationsMs: {
    tailwag: 100,
    "ear-perk": 100,
    twirl: 100,
    stretch: 100,
    chase: 100,
    pickup: 100,
    "carry-away": 100,
    skateboard: 100,
    bark: 100,
    "reading-enter": 100,
    "reading-exit": 100,
    feeding: 100,
    full: 100,
  },
};

let hidden: boolean;
let reducedMotion: boolean;
let media: MediaQueryList;
const runtime = {
  now: () => performance.now(),
  random: vi.fn(() => 0.99),
};

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
  hidden = false;
  reducedMotion = false;
  runtime.random.mockClear();
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  media = Object.assign(new EventTarget(), {
    media: "(prefers-reduced-motion: reduce)",
    onchange: null,
  }) as MediaQueryList;
  Object.defineProperty(media, "matches", { get: () => reducedMotion });
  vi.stubGlobal("matchMedia", () => media);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

function advance(ms: number) {
  act(() => vi.advanceTimersByTime(ms));
}

describe("useLuciaBehavior", () => {
  it("rearms an early browser timer against its fractional deadline", () => {
    let now = 0.25;
    const clock = { now: () => now, random: () => 0.99 };
    const { result, unmount } = renderHook(() => useLuciaBehavior(config, "fixture-client", clock));
    now = 99.75;
    advance(100);
    expect(result.current.state.behavior).toBe("tailwag");
    expect(vi.getTimerCount()).toBe(1);
    now = 100.25;
    advance(1);
    expect(result.current.state.behavior).toBe("idle");
    expect(vi.getTimerCount()).toBe(1);
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("owns one scheduled deadline and cancels all work on unmount", () => {
    const { result, unmount } = renderHook(() => useLuciaBehavior(config, "fixture-client", runtime));
    expect(result.current.state.behavior).toBe("tailwag");
    expect(vi.getTimerCount()).toBe(1);
    advance(100);
    expect(result.current.state.behavior).toBe("idle");
    expect(vi.getTimerCount()).toBe(1);

    const removeDocumentListener = vi.spyOn(document, "removeEventListener");
    const removeMediaListener = vi.spyOn(media, "removeEventListener");
    unmount();
    expect(vi.getTimerCount()).toBe(0);
    expect(removeDocumentListener).toHaveBeenCalledWith("input", expect.any(Function), true);
    expect(removeDocumentListener).toHaveBeenCalledWith("visibilitychange", expect.any(Function));
    expect(removeMediaListener).toHaveBeenCalledWith("change", expect.any(Function));
    runtime.random.mockClear();
    advance(600_000);
    expect(runtime.random).not.toHaveBeenCalled();
  });

  it("reacts to editing activity without handling navigation keys or changing focus", () => {
    const { result } = renderHook(() => useLuciaBehavior(config, "fixture-client", runtime));
    const input = document.createElement("textarea");
    document.body.append(input);
    input.focus();
    advance(100);
    advance(30_000);
    const beforeNavigation = result.current.state;
    fireEvent.keyDown(input, { key: "Tab" });
    fireEvent.keyDown(input, { key: "ArrowLeft" });
    expect(result.current.state).toBe(beforeNavigation);

    fireEvent.input(input);
    expect(result.current.state.behavior).toBe("ear-perk");
    expect(document.activeElement).toBe(input);
    advance(100);
    fireEvent.input(input);
    expect(result.current.state.behavior).toBe("idle");
    expect(document.activeElement).toBe(input);
  });

  it("ignores non-writing controls and password fields", () => {
    const { result } = renderHook(() => useLuciaBehavior(config, "fixture-client", runtime));
    advance(100);
    advance(30_000);
    const before = result.current.state;
    for (const type of ["checkbox", "range", "password"]) {
      const input = document.createElement("input");
      input.type = type;
      document.body.append(input);
      fireEvent.input(input);
    }
    expect(result.current.state).toBe(before);
  });

  it("treats the document's contenteditable surface as writing", () => {
    const { result } = renderHook(() => useLuciaBehavior(config, "fixture-client", runtime));
    const editor = document.createElement("div");
    editor.contentEditable = "true";
    document.body.append(editor);
    advance(100);
    advance(30_000);
    fireEvent.input(editor);
    expect(result.current.state.behavior).toBe("ear-perk");
  });

  it("drops hidden work without accumulating a replay queue", () => {
    const { result } = renderHook(() => useLuciaBehavior(config, "fixture-client", runtime));
    act(() => result.current.dropSock());
    expect(result.current.state.behavior).toBe("chase");
    hidden = true;
    fireEvent(document, new Event("visibilitychange"));
    expect(result.current.state.behavior).toBe("inactive");
    expect(vi.getTimerCount()).toBe(0);
    advance(600_000);
    expect(result.current.state.behavior).toBe("inactive");
    hidden = false;
    fireEvent(document, new Event("visibilitychange"));
    expect(result.current.state.behavior).toBe("tailwag");
    expect(vi.getTimerCount()).toBe(1);
  });

  it("honors initial and changed reduced-motion preferences", () => {
    reducedMotion = true;
    const { result } = renderHook(() => useLuciaBehavior(config, "fixture-client", runtime));
    expect(result.current.state.behavior).toBe("idle");
    expect(vi.getTimerCount()).toBe(0);

    reducedMotion = false;
    act(() => media.dispatchEvent(new Event("change")));
    expect(result.current.state.reducedMotion).toBe(false);
    expect(vi.getTimerCount()).toBe(1);
    act(() => result.current.dropSock());
    reducedMotion = true;
    act(() => media.dispatchEvent(new Event("change")));
    expect(result.current.state.reducedMotion).toBe(true);
    // Any explicit sock state may settle statically; hidden always cancels it.
    hidden = true;
    fireEvent(document, new Event("visibilitychange"));
    expect(vi.getTimerCount()).toBe(0);
  });

  it("keeps one reading episode for overlapping requests in the current client", async () => {
    let finishFirst!: () => void;
    let finishSecond!: () => void;
    const first = withLuciaInference("fixture-client", () => new Promise<void>((resolve) => {
      finishFirst = resolve;
    }));
    const { result } = renderHook(() => useLuciaBehavior(config, "fixture-client", runtime));
    advance(199);
    expect(result.current.state.behavior).not.toBe("reading-enter");
    advance(1);
    expect(result.current.state.behavior).toBe("reading-enter");
    let second!: Promise<void>;
    act(() => {
      second = withLuciaInference("fixture-client", () => new Promise<void>((resolve) => {
        finishSecond = resolve;
      }));
    });
    await act(async () => {
      finishFirst();
      await first;
    });
    advance(100);
    expect(result.current.state.behavior).toBe("reading");
    expect(vi.getTimerCount()).toBe(0);
    await act(async () => {
      finishSecond();
      await second;
    });
    expect(result.current.state.behavior).toBe("reading-exit");
    advance(100);
    expect(result.current.state.behavior).toBe("idle");
  });

  it("ignores another client's request and cannot restart after unmount", async () => {
    const { result, unmount } = renderHook(() => useLuciaBehavior(config, "fixture-client", runtime));
    let finish!: () => void;
    const request = withLuciaInference("different-client", () => new Promise<void>((resolve) => {
      finish = resolve;
    }));
    advance(100);
    advance(200);
    expect(result.current.state.behavior).toBe("idle");
    unmount();
    await act(async () => {
      finish();
      await request;
    });
    expect(getLuciaInferenceCount("different-client")).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });
});
