import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import LuciaCompanion from "./LuciaCompanion";
import { LUCIA_BEHAVIOR_CONFIG as config, loadLuciaArtwork } from "../lib/luciaArtwork";
import { withLuciaInference } from "../lib/luciaInference";

vi.mock("../lib/luciaArtwork", async (original) => ({
  ...await original<typeof import("../lib/luciaArtwork")>(),
  loadLuciaArtwork: vi.fn(async () => true),
}));

let rail: HTMLDivElement;
let hidden = false;
let reduced = false;
let media: MediaQueryList;
const advance = (ms: number) => act(() => vi.advanceTimersByTime(ms));

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame", "performance"] });
  hidden = false;
  reduced = false;
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  media = Object.assign(new EventTarget(), { media: "(prefers-reduced-motion: reduce)", onchange: null }) as MediaQueryList;
  Object.defineProperty(media, "matches", { get: () => reduced });
  vi.stubGlobal("matchMedia", () => media);
  vi.mocked(loadLuciaArtwork).mockResolvedValue(true);
  rail = document.createElement("div");
  Object.defineProperty(rail, "clientWidth", { value: 1000 });
  document.body.append(rail);
});
afterEach(() => {
  rail.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
async function mount() {
  let result!: ReturnType<typeof render>;
  await act(async () => { result = render(<LuciaCompanion clientId="fixture" rail={rail} />); });
  return result;
}
function mode(value: string) { fireEvent.change(screen.getByLabelText("Lucia mode"), { target: { value } }); }
function scene() { return screen.getByTestId("lucia-scene"); }
function sprite() { return screen.getByTestId("lucia-sprite"); }

describe("LuciaCompanion", () => {
  it("hides the standalone sock at pickup START, with no duplicate during carry", async () => {
    const { unmount } = await mount();
    fireEvent.click(screen.getByRole("button", { name: "Drop a sock for Lucia" }));
    expect(scene().dataset.behavior).toBe("chase");
    expect(screen.getByTestId("lucia-sock")).toBeDefined();
    advance(config.durationsMs.chase);
    expect(scene().dataset.behavior).toBe("pickup");
    expect(screen.queryByTestId("lucia-sock")).toBeNull();
    expect(sprite().dataset.image).toContain("pickup");
    advance(config.durationsMs.pickup);
    expect(sprite().dataset.image).toContain("carry-right");
    expect(screen.queryByTestId("lucia-sock")).toBeNull();
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("retains the current static body on later feeds and commits once, never a sixth fruit", async () => {
    await mount();
    mode("feeding");
    const feed = screen.getByRole("button", { name: "Feed a dekopon", hidden: true });
    for (let count = 0; count < 5; count++) {
      fireEvent.click(feed);
      fireEvent.click(feed);
      expect(scene().dataset.fedCount).toBe(String(count));
      expect(sprite().dataset.image).toContain(count === 0 ? "feeding-strip" : "stages-strip");
      if (count > 0) expect(sprite().dataset.frame).toBe(`0:${count}`);
      advance(config.durationsMs.feeding);
      expect(scene().dataset.fedCount).toBe(String(count + 1));
    }
    expect(scene().dataset.behavior).toBe("full");
    advance(config.durationsMs.full);
    expect(sprite().dataset.frame).toBe("0:5");
    expect((feed as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(feed);
    advance(2000);
    expect(scene().dataset.fedCount).toBe("5");
    fireEvent.click(screen.getByRole("button", { name: "Reset feeding", hidden: true }));
    expect(scene().dataset.fedCount).toBe("0");
  });

  it("keeps reading for overlapping jobs and exits after failure/cancellation settlement", async () => {
    await mount();
    let finish!: () => void;
    let fail!: (reason: Error) => void;
    let one!: Promise<void>;
    let two!: Promise<void>;
    await act(async () => {
      one = withLuciaInference("fixture", () => new Promise<void>((resolve) => { finish = resolve; }));
      two = withLuciaInference("fixture", () => new Promise<void>((_, reject) => { fail = reject; }));
    });
    advance(200);
    expect(scene().dataset.behavior).toBe("reading-enter");
    advance(config.durationsMs["reading-enter"]);
    await act(async () => { finish(); await one; });
    expect(scene().dataset.behavior).toBe("reading");
    await act(async () => { fail(new Error("fixture cancellation")); await expect(two).rejects.toThrow("fixture cancellation"); });
    expect(scene().dataset.behavior).toBe("reading-exit");
    advance(config.durationsMs["reading-exit"]);
    expect(scene().dataset.behavior).toBe("idle");
  });

  it("cleans input, gaze and animation on hidden window and repeated unmounts", async () => {
    for (let i = 0; i < 3; i++) {
      const { unmount } = await mount();
      mode("gaze");
      mode("feeding");
      mode("gaze");
      hidden = true;
      fireEvent(document, new Event("visibilitychange"));
      expect(screen.queryByTestId("lucia-scene")).toBeNull();
      expect(vi.getTimerCount()).toBe(0);
      advance(600_000);
      hidden = false;
      fireEvent(document, new Event("visibilitychange"));
      expect(scene().dataset.behavior).toBe("tailwag");
      const remove = vi.spyOn(window, "removeEventListener");
      unmount();
      expect(vi.getTimerCount()).toBe(0);
      expect(remove).toHaveBeenCalledWith("pointermove", expect.any(Function));
    }
  });

  it("holds static artwork with no ambient timers in reduced motion", async () => {
    reduced = true;
    const { unmount } = await mount();
    const frame = sprite().dataset.frame;
    advance(600_000);
    expect(sprite().dataset.frame).toBe(frame);
    expect(vi.getTimerCount()).toBe(0);
    mode("gaze");
    expect(vi.getTimerCount()).toBe(0);
    mode("feeding");
    fireEvent.click(screen.getByRole("button", { name: "Feed a dekopon", hidden: true }));
    expect(sprite().dataset.image).toContain("stages-strip");
    advance(config.durationsMs.feeding);
    expect(sprite().dataset.frame).toBe("0:1");
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("exposes failed image load without mounting listeners, animation or click interception", async () => {
    vi.mocked(loadLuciaArtwork).mockRejectedValue(new Error("Lucia artwork unavailable"));
    const { unmount } = await mount();
    expect(screen.getByRole("alert").textContent).toContain("Lucia artwork unavailable");
    expect(screen.queryByTestId("lucia-scene")).toBeNull();
    expect(screen.queryByRole("button", { name: "Drop a sock for Lucia" })).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
    unmount();
  });
});
