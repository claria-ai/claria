import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SockDrop from "./SockDrop";
import {
  DOG_FRAME_A,
  DOG_FRAME_B,
  DOG_HEAD,
  DOG_PALETTE,
  DOG_PLAY_BOW,
  SOCK_DROP_TIMINGS,
  SOCK_MAP,
  SOCK_PALETTE,
  randomShakeCount,
} from "../lib/sockDrop";

/**
 * The phase state machine is the real subject here: it is timeout-driven
 * precisely so this file can walk through it with fake timers. The CSS
 * keyframes it triggers are visual-only and untestable outside a browser.
 */

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

const T = SOCK_DROP_TIMINGS;

function advance(ms: number) {
  act(() => {
    vi.advanceTimersByTime(ms);
  });
}

function phase(): string | undefined {
  return screen.getByTestId("sock-drop").dataset.phase;
}

describe("SockDrop", () => {
  it("runs drop → walk-in → grab → shake → walk-off, then clears", () => {
    const onDone = vi.fn();
    render(<SockDrop onDone={onDone} shakeCount={5} />);

    expect(phase()).toBe("drop");
    expect(screen.getByTestId("sock-standing")).toBeTruthy();
    expect(screen.queryByTestId("sock-carried")).toBeNull();

    advance(T.dropMs);
    expect(phase()).toBe("walk-in");

    advance(T.walkInMs);
    expect(phase()).toBe("grab");
    // The moment she grabs it, the sock moves from the floor to her mouth and
    // her body plants in a play bow.
    expect(screen.queryByTestId("sock-standing")).toBeNull();
    expect(screen.getByTestId("sock-carried")).toBeTruthy();
    expect(screen.getByTestId("sock-dog-body").dataset.pose).toBe("play-bow");

    advance(T.grabMs);
    expect(phase()).toBe("shake");
    expect(screen.getByTestId("sock-dog-motion").style.animation).toBe("");
    expect(screen.getByTestId("sock-dog-neck").style.animation).toContain(
      "sock-dog-neck-shake"
    );

    advance(T.shakeMsPerShake * 5);
    expect(phase()).toBe("walk-off");
    expect(screen.getByTestId("sock-carried")).toBeTruthy();
    expect(screen.getByTestId("sock-dog-body").dataset.pose).toBe("standing");
    expect(onDone).not.toHaveBeenCalled();

    advance(T.walkOffMs);
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("shake duration scales with the shake count", () => {
    const onDone = vi.fn();
    render(<SockDrop onDone={onDone} shakeCount={8} />);
    // Each phase's timer is armed by an effect after the previous one fires,
    // so the clock has to be advanced phase by phase.
    advance(T.dropMs);
    advance(T.walkInMs);
    advance(T.grabMs);
    expect(phase()).toBe("shake");

    // Five shakes in — still shaking.
    advance(T.shakeMsPerShake * 5);
    expect(phase()).toBe("shake");
    advance(T.shakeMsPerShake * 3);
    expect(phase()).toBe("walk-off");
  });

  it("picks a random shake count between 3 and 8 by default", () => {
    render(<SockDrop onDone={() => {}} />);
    const shakes = Number(screen.getByTestId("sock-drop").dataset.shakes);
    expect(Number.isInteger(shakes)).toBe(true);
    expect(shakes).toBeGreaterThanOrEqual(3);
    expect(shakes).toBeLessThanOrEqual(8);
  });

  it("does not fire onDone after unmount", () => {
    const onDone = vi.fn();
    const { unmount } = render(<SockDrop onDone={onDone} shakeCount={3} />);
    unmount();
    advance(
      T.dropMs + T.walkInMs + T.grabMs + T.shakeMsPerShake * 3 + T.walkOffMs
    );
    expect(onDone).not.toHaveBeenCalled();
  });
});

describe("randomShakeCount", () => {
  it("always lands on an integer from 3 to 8", () => {
    for (let i = 0; i < 500; i++) {
      const n = randomShakeCount();
      expect(Number.isInteger(n)).toBe(true);
      expect(n).toBeGreaterThanOrEqual(3);
      expect(n).toBeLessThanOrEqual(8);
    }
  });

  it("covers both endpoints", () => {
    const spy = vi.spyOn(Math, "random");
    spy.mockReturnValue(0);
    expect(randomShakeCount()).toBe(3);
    spy.mockReturnValue(0.9999999);
    expect(randomShakeCount()).toBe(8);
    spy.mockRestore();
  });
});

describe("pixel sprites", () => {
  // A ragged row or an unpainted non-transparent character would silently
  // distort the artwork, so the maps are validated as data.
  it.each([
    ["sock", SOCK_MAP, SOCK_PALETTE],
    ["dog frame A", DOG_FRAME_A, DOG_PALETTE],
    ["dog frame B", DOG_FRAME_B, DOG_PALETTE],
    ["dog play bow", DOG_PLAY_BOW, DOG_PALETTE],
    ["dog head", DOG_HEAD, DOG_PALETTE],
  ])("%s map is rectangular and only uses palette colors", (_name, map, palette) => {
    const width = map[0].length;
    for (const row of map) {
      expect(row.length).toBe(width);
      for (const ch of row) {
        if (ch !== ".") {
          expect(palette[ch]).toBeDefined();
        }
      }
    }
  });

  it("dog frames differ only in the legs", () => {
    expect(DOG_FRAME_A.length).toBe(DOG_FRAME_B.length);
    // Shared body rows are identical; at least one leg row differs.
    expect(DOG_FRAME_A.slice(0, 20)).toEqual(DOG_FRAME_B.slice(0, 20));
    expect(DOG_FRAME_A.slice(20)).not.toEqual(DOG_FRAME_B.slice(20));
  });

  it("uses a detailed, graduated palette with a strong white moustache", () => {
    const paintedHead = DOG_HEAD.join("").replaceAll(".", "");
    const usedDogColors = new Set(
      [...DOG_FRAME_A, ...DOG_PLAY_BOW, ...DOG_HEAD]
        .join("")
        .replaceAll(".", "")
    );
    expect(paintedHead.length).toBeGreaterThan(250);
    expect(usedDogColors.size).toBeGreaterThanOrEqual(14);
    expect((paintedHead.match(/W/g) ?? []).length).toBeGreaterThan(30);
  });
});
