import manifest from "../assets/lucia/manifest.json";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import LuciaCompanion from "./LuciaCompanion";
import { LUCIA_BEHAVIOR_CONFIG as config, loadLuciaArtwork } from "../lib/luciaArtwork";
import { withLuciaInference } from "../lib/luciaInference";

vi.mock("../lib/luciaArtwork", async (original) => ({
  ...await original<typeof import("../lib/luciaArtwork")>(),
  loadLuciaArtwork: vi.fn(),
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
  vi.mocked(loadLuciaArtwork).mockResolvedValue(new Map(Object.keys(manifest.assets).map((path) => [path, new Image()])));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D);
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
async function mount(previewsEnabled = false) {
  let result!: ReturnType<typeof render>;
  await act(async () => { result = render(<LuciaCompanion clientId="fixture" rail={rail} previewsEnabled={previewsEnabled} />); });
  return result;
}
const MODE_LABELS: Record<string, string> = { companion: "Companion", gaze: "Watch", feeding: "Feed" };
function mode(value: string) { fireEvent.click(screen.getByRole("button", { name: MODE_LABELS[value], pressed: false })); }
function scene() { return screen.getByTestId("lucia-scene"); }
function sprite() { return screen.getByTestId("lucia-sprite"); }

describe("LuciaCompanion", () => {
  it("hides the standalone sock at pickup START, with no duplicate during carry", async () => {
    const { unmount } = await mount();
    const canvas = sprite().querySelector("canvas")!;
    const context = canvas.getContext("2d")!;
    const sources = await vi.mocked(loadLuciaArtwork).mock.results[0].value;
    fireEvent.click(screen.getByRole("button", { name: "Drop a sock for Lucia" }));
    expect(context.drawImage).toHaveBeenLastCalledWith(sources.get(manifest.core_image), 0, 208, 192, 208, 0, 0, 192, 208);
    expect(scene().dataset.behavior).toBe("chase");
    expect(screen.getByTestId("lucia-sock")).toBeDefined();
    advance(config.durationsMs.chase);
    expect(scene().dataset.behavior).toBe("pickup");
    expect(screen.queryByTestId("lucia-sock")).toBeNull();
    expect(sprite().dataset.image).toContain("pickup");
    expect(sprite().querySelector("canvas")).toBe(canvas);
    expect(context.drawImage).toHaveBeenLastCalledWith(sources.get(manifest.clips.pickup.image), 0, 0, 192, 208, 0, 0, 192, 208);
    expect(sprite().dataset.frame).toBe("0:0");
    advance(config.durationsMs.pickup);
    expect(sprite().dataset.image).toContain("carry-right");
    expect(sprite().querySelector("canvas")).toBe(canvas);
    expect(context.drawImage).toHaveBeenLastCalledWith(sources.get(manifest.clips["carry-away"].image), 0, 0, 192, 208, 0, 0, 192, 208);
    expect(sprite().dataset.frame).toBe("0:0");
    expect(sprite().style.width).toBe("124.8px");
    expect(parseFloat(sprite().style.height)).toBeCloseTo(135.2);
    expect(screen.queryByTestId("lucia-sock")).toBeNull();
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("loops idle in place, drawing only when the cell changes and never moving its registration", async () => {
    await mount();
    advance(config.durationsMs.tailwag);
    const context = sprite().querySelector("canvas")!.getContext("2d")!;
    const calls = vi.mocked(context.drawImage).mock.calls.length;
    const registration = [sprite().style.left, sprite().style.bottom];
    const seen = new Set<string>();
    for (let step = 0; step < 50; step++) {
      advance(110);
      seen.add(sprite().dataset.frame!);
      expect([sprite().style.left, sprite().style.bottom]).toEqual(registration);
    }
    expect(scene().dataset.behavior).toBe("idle");
    expect([...seen].sort()).toEqual(["0:0", "0:1", "0:2", "0:3", "0:4", "0:5"]);
    // 5.5s is five 1100ms cycles: six cell changes each, not one draw per display frame.
    expect(vi.mocked(context.drawImage).mock.calls.length - calls).toBeLessThanOrEqual(31);
    expect(screen.queryByLabelText("Preview Lucia action")).toBeNull();
  });

  it("offers explicit local previews only when opted in and respects action ownership", async () => {
    await mount(true);
    const preview = screen.getByLabelText("Preview Lucia action") as HTMLSelectElement;
    fireEvent.change(preview, { target: { value: "stretch" } });
    expect(scene().dataset.behavior).toBe("stretch");
    advance(config.durationsMs.stretch);
    expect(scene().dataset.behavior).toBe("bored");
    fireEvent.change(preview, { target: { value: "twirl" } });
    expect(scene().dataset.behavior).toBe("twirl");
    expect(sprite().dataset.image).toContain("twirl");
    fireEvent.click(screen.getByRole("button", { name: "Drop a sock for Lucia" }));
    expect(preview.disabled).toBe(true);
    fireEvent.change(preview, { target: { value: "stretch" } });
    expect(scene().dataset.behavior).toBe("chase");
  });

  it("does not mount a drawing surface or scene until decoding finishes", async () => {
    let finish!: (artwork: Awaited<ReturnType<typeof loadLuciaArtwork>>) => void;
    vi.mocked(loadLuciaArtwork).mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    await mount();
    expect(screen.queryByTestId("lucia-scene")).toBeNull();
    expect(screen.getByRole("status").textContent).toContain("Loading");
    await act(async () => { finish(new Map(Object.keys(manifest.assets).map((path) => [path, new Image()]))); });
    expect(sprite().querySelector("canvas")).not.toBeNull();
  });

  it("animates the matching current body for every feed and commits once, never a sixth fruit", async () => {
    await mount();
    mode("feeding");
    const feed = screen.getByRole("button", { name: "Feed a dekopon", hidden: true });
    for (let count = 0; count < 5; count++) {
      fireEvent.click(feed);
      fireEvent.click(feed);
      expect(scene().dataset.fedCount).toBe(String(count));
      expect(sprite().dataset.image).toBe(`extras/dekopon/${count === 0 ? "feeding" : `feeding-size-${count}`}/feeding-strip.png`);
      expect(sprite().dataset.frame).toBe("0:0");
      expect(screen.queryByTestId("lucia-fruit")).toBeNull();
      advance(800);
      expect(sprite().dataset.frame).toBe("0:5");
      expect(scene().dataset.fedCount).toBe(String(count));
      advance(config.durationsMs.feeding - 800);
      expect(scene().dataset.fedCount).toBe(String(count + 1));
    }
    expect(scene().dataset.behavior).toBe("full");
    advance(config.durationsMs.full);
    expect(sprite().dataset.frame).toBe("0:5");
    expect((feed as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(feed);
    advance(2000);
    expect(scene().dataset.fedCount).toBe("5");
    fireEvent.click(screen.getByRole("button", { name: "Start over", hidden: true }));
    expect(scene().dataset.fedCount).toBe("0");
  });

  it.each([1, 2, 3, 4])("cancels size %i feeding on hide without committing another fruit", async (count) => {
    await mount();
    mode("feeding");
    const feed = screen.getByRole("button", { name: "Feed a dekopon", hidden: true });
    for (let completed = 0; completed < count; completed++) {
      fireEvent.click(feed);
      advance(config.durationsMs.feeding);
    }
    fireEvent.click(feed);
    advance(300);
    hidden = true;
    fireEvent(document, new Event("visibilitychange"));
    advance(config.durationsMs.feeding * 2);
    expect(screen.queryByTestId("lucia-scene")).toBeNull();
    hidden = false;
    fireEvent(document, new Event("visibilitychange"));
    expect(scene().dataset.fedCount).toBe(String(count));
    expect(sprite().dataset.image).toBe(manifest.stages.image);
    expect(sprite().dataset.frame).toBe(`0:${count}`);
  });

  it("keeps reading for overlapping jobs and exits after failure/cancellation settlement", async () => {
    await mount(true);
    const preview = screen.getByLabelText("Preview Lucia action") as HTMLSelectElement;
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
    expect(preview.disabled).toBe(true);
    advance(config.durationsMs["reading-exit"]);
    expect(scene().dataset.behavior).toBe("idle");
    expect(preview.disabled).toBe(false);
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
    for (let count = 0; count < 5; count++) {
      fireEvent.click(screen.getByRole("button", { name: "Feed a dekopon", hidden: true }));
      expect(sprite().dataset.image).toBe(manifest.stages.image);
      expect(sprite().dataset.frame).toBe(`0:${count}`);
      advance(config.durationsMs.feeding);
      expect(scene().dataset.fedCount).toBe(String(count + 1));
    }
    advance(config.durationsMs.full);
    expect(sprite().dataset.frame).toBe("0:5");
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("sends Lucia across the rail on her skateboard from the board button", async () => {
    await mount();
    advance(config.durationsMs.tailwag);
    const board = screen.getByRole("button", { name: "Give Lucia her skateboard" }) as HTMLButtonElement;
    fireEvent.click(board);
    expect(scene().dataset.behavior).toBe("skateboard");
    expect(sprite().dataset.image).toBe(manifest.clips.skateboard.image);
    expect(board.disabled).toBe(true);
    advance(config.durationsMs.skateboard);
    expect(scene().dataset.behavior).toBe("idle");
    expect(board.disabled).toBe(false);
    mode("feeding");
    expect(screen.queryByRole("button", { name: "Give Lucia her skateboard" })).toBeNull();
  });

  it("leaves the skateboard button out under reduced motion", async () => {
    reduced = true;
    await mount();
    expect(screen.getByRole("button", { name: "Drop a sock for Lucia" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Give Lucia her skateboard" })).toBeNull();
  });

  it("ignores clicks on Lucia while a pet reaction plays", async () => {
    await mount();
    advance(config.durationsMs.tailwag);
    fireEvent.click(sprite());
    expect(scene().dataset.behavior).toBe("bark");
    expect((sprite() as HTMLButtonElement).disabled).toBe(true);
    advance(config.durationsMs.bark);
    expect((sprite() as HTMLButtonElement).disabled).toBe(false);
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

  it("feeds when Lucia herself is clicked in feeding mode, and never offers a dead click", async () => {
    await mount();
    expect(sprite().getAttribute("aria-label")).toBe("Pet Lucia");
    mode("feeding");
    expect(sprite().getAttribute("aria-label")).toBe("Feed Lucia a dekopon");
    for (let count = 0; count < 5; count++) {
      fireEvent.click(sprite());
      expect(scene().dataset.behavior).toBe("feeding");
      advance(config.durationsMs.feeding);
    }
    expect(scene().dataset.fedCount).toBe("5");
    advance(config.durationsMs.full);
    expect(sprite().getAttribute("aria-label")).toBe("Lucia is full");
    expect((sprite() as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows a paint failure in the rail instead of throwing from the animation frame", async () => {
    const partial = new Map(Object.keys(manifest.assets)
      .filter((path) => path !== manifest.stages.image).map((path) => [path, new Image()]));
    vi.mocked(loadLuciaArtwork).mockResolvedValue(partial);
    await mount();
    mode("feeding");
    expect(screen.getByRole("alert").textContent).toContain("Lucia artwork was not decoded");
    expect(screen.queryByTestId("lucia-scene")).toBeNull();
    expect(screen.getByRole("button", { name: "Hide Lucia" })).toBeDefined();
  });

  it("sleeps between frames while resting instead of running a display-rate loop", async () => {
    await mount();
    advance(config.durationsMs.tailwag);
    advance(30_000);
    advance(config.durationsMs.stretch);
    expect(scene().dataset.behavior).toBe("bored");
    const frames = vi.spyOn(window, "requestAnimationFrame");
    advance(1_000);
    // The bored strip changes cell about every 150ms; a display-rate loop would ask ~60 times.
    expect(frames.mock.calls.length).toBeLessThanOrEqual(10);
    expect(frames.mock.calls.length).toBeGreaterThan(0);
  });
});
