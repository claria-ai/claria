import { createRef } from "react";
import { act, cleanup, fireEvent, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useLuciaGaze } from "./useLuciaGaze";

const headAnchor = { x: 96, y: 70 };
const settings = { paused: false, reducedMotion: false, headAnchor };
let hidden: boolean;
let frames: Map<number, FrameRequestCallback>;
let nextFrame: number;

beforeEach(() => {
  hidden = false;
  frames = new Map();
  nextFrame = 0;
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
    const id = ++nextFrame;
    frames.set(id, callback);
    return id;
  }));
  vi.stubGlobal("cancelAnimationFrame", vi.fn((id: number) => frames.delete(id)));
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function target() {
  const ref = createRef<HTMLDivElement>();
  ref.current = document.createElement("div");
  ref.current.style.left = "100px";
  ref.current.style.top = "100px";
  vi.spyOn(ref.current, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 100, 192, 208));
  return ref;
}

function move(x: number, y: number, pointerType = "mouse") {
  fireEvent.pointerMove(window, { clientX: x, clientY: y, pointerType });
}

function paint() {
  const callbacks = [...frames.values()];
  frames.clear();
  act(() => callbacks.forEach((callback) => callback(performance.now())));
}

describe("useLuciaGaze", () => {
  it.each([
    ["up", 196, 70, 9, 0],
    ["right", 296, 170, 9, 4],
    ["down", 196, 270, 10, 0],
    ["left", 96, 170, 10, 4],
  ])("looks %s from the measured head center without moving the seated sprite", (_name, x, y, row, column) => {
    const ref = target();
    const { result } = renderHook(() => useLuciaGaze(ref, settings));
    move(Number(x), Number(y));
    paint();
    expect(result.current).toMatchObject({ row, column });
    expect(ref.current!.style.left).toBe("100px");
    expect(ref.current!.style.top).toBe("100px");
    expect(ref.current!.style.transform).toBe("");
    expect(frames.size).toBe(0);
  });

  it("coalesces pointer events and only publishes a changed direction", () => {
    const ref = target();
    const { result } = renderHook(() => useLuciaGaze(ref, settings));
    move(196, 70);
    move(296, 170);
    move(297, 170);
    expect(frames.size).toBe(1);
    paint();
    const right = result.current;
    expect(right).toMatchObject({ row: 9, column: 4 });
    move(298, 170);
    paint();
    expect(result.current).toBe(right);
    expect(frames.size).toBe(0);
  });

  it("uses an inclusive display-pixel deadzone and recalculates after resizing", () => {
    const ref = target();
    const { result } = renderHook(() => useLuciaGaze(ref, settings));
    move(220, 170);
    paint();
    expect(result.current).toMatchObject({ row: 0, column: 0 });
    move(221, 170);
    paint();
    expect(result.current).toMatchObject({ row: 9, column: 4 });

    vi.mocked(ref.current!.getBoundingClientRect).mockReturnValue(new DOMRect(125, 100, 192, 208));
    fireEvent(window, new Event("resize"));
    paint();
    expect(result.current).toMatchObject({ row: 0, column: 0 });
    vi.mocked(ref.current!.getBoundingClientRect).mockReturnValue(new DOMRect(100, 100, 384, 416));
    move(392, 240);
    paint();
    expect(result.current).toMatchObject({ row: 9, column: 4 });
  });

  it("pauses for another behavior or reduced motion and resumes the seated direction", () => {
    const ref = target();
    const { result, rerender } = renderHook((props) => useLuciaGaze(ref, props), { initialProps: settings });
    move(296, 170);
    paint();
    expect(result.current).toMatchObject({ row: 9, column: 4 });
    rerender({ ...settings, paused: true });
    expect(result.current).toMatchObject({ row: 0, column: 0 });
    move(196, 270);
    expect(frames.size).toBe(0);
    rerender(settings);
    paint();
    expect(result.current).toMatchObject({ row: 10, column: 0 });
    move(196, 70);
    expect(frames.size).toBe(1);
    rerender({ ...settings, reducedMotion: true });
    expect(frames.size).toBe(0);
    expect(result.current).toMatchObject({ row: 0, column: 0 });
    move(196, 270);
    expect(frames.size).toBe(0);
    rerender(settings);
    paint();
    expect(result.current).toMatchObject({ row: 10, column: 0 });
  });

  it("starts neutral after exiting and reentering gaze mode", () => {
    const ref = target();
    const first = renderHook(() => useLuciaGaze(ref, settings));
    move(296, 170);
    paint();
    expect(first.result.current).toMatchObject({ row: 9, column: 4 });
    first.unmount();
    move(196, 270);
    expect(frames.size).toBe(0);
    const second = renderHook(() => useLuciaGaze(ref, settings));
    expect(second.result.current).toMatchObject({ row: 0, column: 0 });
    expect(frames.size).toBe(0);
    move(196, 270);
    paint();
    expect(second.result.current).toMatchObject({ row: 10, column: 0 });
  });

  it("cancels a queued frame while hidden and never starts an idle polling loop", () => {
    const ref = target();
    const { result } = renderHook(() => useLuciaGaze(ref, settings));
    move(296, 170);
    expect(frames.size).toBe(1);
    hidden = true;
    fireEvent(document, new Event("visibilitychange"));
    expect(frames.size).toBe(0);
    move(196, 70);
    expect(frames.size).toBe(0);
    hidden = false;
    fireEvent(document, new Event("visibilitychange"));
    paint();
    expect(result.current).toMatchObject({ row: 9, column: 0 });
    expect(frames.size).toBe(0);
  });

  it("returns to neutral when the pointer leaves and ignores pen and touch movement", () => {
    const ref = target();
    const { result } = renderHook(() => useLuciaGaze(ref, settings));
    move(296, 170, "touch");
    move(296, 170, "pen");
    expect(frames.size).toBe(0);
    move(296, 170);
    paint();
    fireEvent.pointerLeave(document);
    expect(result.current).toMatchObject({ row: 0, column: 0 });
    expect(frames.size).toBe(0);
  });

  it("releases listeners and queued animation frames on unmount", () => {
    const ref = target();
    const { unmount } = renderHook(() => useLuciaGaze(ref, settings));
    const removeWindow = vi.spyOn(window, "removeEventListener");
    const removeDocument = vi.spyOn(document, "removeEventListener");
    move(296, 170);
    unmount();
    expect(frames.size).toBe(0);
    expect(removeWindow).toHaveBeenCalledWith("pointermove", expect.any(Function));
    expect(removeWindow).toHaveBeenCalledWith("resize", expect.any(Function));
    expect(removeWindow).toHaveBeenCalledWith("scroll", expect.any(Function), true);
    expect(removeWindow).toHaveBeenCalledWith("blur", expect.any(Function));
    expect(removeDocument).toHaveBeenCalledWith("pointerleave", expect.any(Function));
    expect(removeDocument).toHaveBeenCalledWith("visibilitychange", expect.any(Function));
    move(196, 70);
    expect(frames.size).toBe(0);
  });
});
