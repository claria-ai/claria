import { StrictMode, createElement } from "react";
import { act, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWebviewFileDrop } from "./useWebviewFileDrop";

/**
 * The registration race is the whole reason this hook exists, so it is tested
 * against the shape of the API that causes it: `onDragDropEvent` hands back a
 * *Promise* of the unlisten function, which can resolve after React has
 * already torn the effect down.
 */

type DropPayload =
  | { type: "enter" | "over" | "leave" }
  | { type: "drop"; paths: string[] };

type Listener = (event: { payload: DropPayload }) => void;

const registrations: {
  listener: Listener;
  resolve: () => void;
  unlisten: ReturnType<typeof vi.fn>;
}[] = [];

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (listener: Listener) => {
      const unlisten = vi.fn();
      return new Promise<() => void>((resolvePromise) => {
        registrations.push({
          listener,
          unlisten,
          resolve: () => resolvePromise(unlisten),
        });
      });
    },
  }),
}));

beforeEach(() => {
  registrations.length = 0;
});

/** Renders the hook and exposes what it reported. */
function renderHook(
  props: Parameters<typeof useWebviewFileDrop>[0],
  { strict = false } = {},
) {
  const dragging: boolean[] = [];
  function Probe() {
    dragging.push(useWebviewFileDrop(props));
    return null;
  }
  const element = strict
    ? createElement(StrictMode, null, createElement(Probe))
    : createElement(Probe);
  const view = render(element);
  return { dragging, view };
}

describe("useWebviewFileDrop", () => {
  it("reports dragging across enter, over and leave", async () => {
    const { dragging } = renderHook({ onDrop: vi.fn() });
    await act(async () => registrations[0].resolve());

    await act(async () => {
      registrations[0].listener({ payload: { type: "enter" } });
    });
    expect(dragging.at(-1)).toBe(true);

    await act(async () => {
      registrations[0].listener({ payload: { type: "over" } });
    });
    expect(dragging.at(-1)).toBe(true);

    await act(async () => {
      registrations[0].listener({ payload: { type: "leave" } });
    });
    expect(dragging.at(-1)).toBe(false);
  });

  it("clears dragging and forwards the paths on drop", async () => {
    const onDrop = vi.fn();
    const { dragging } = renderHook({ onDrop });
    await act(async () => registrations[0].resolve());

    await act(async () => {
      registrations[0].listener({ payload: { type: "enter" } });
      registrations[0].listener({ payload: { type: "drop", paths: ["/a.m4a"] } });
    });

    expect(onDrop).toHaveBeenCalledWith(["/a.m4a"]);
    expect(dragging.at(-1)).toBe(false);
  });

  it("lets divert consume a drop before onDrop sees it", async () => {
    const onDrop = vi.fn();
    const divert = vi.fn(() => true);
    renderHook({ onDrop, divert });
    await act(async () => registrations[0].resolve());

    await act(async () => {
      registrations[0].listener({ payload: { type: "drop", paths: ["/a.m4a"] } });
    });

    expect(divert).toHaveBeenCalledWith(["/a.m4a"]);
    expect(onDrop).not.toHaveBeenCalled();
  });

  it("falls through to onDrop when divert declines", async () => {
    const onDrop = vi.fn();
    renderHook({ onDrop, divert: () => false });
    await act(async () => registrations[0].resolve());

    await act(async () => {
      registrations[0].listener({ payload: { type: "drop", paths: ["/a.m4a"] } });
    });

    expect(onDrop).toHaveBeenCalledWith(["/a.m4a"]);
  });

  it("drains a listener that registers after cleanup, leaving exactly one live", async () => {
    // StrictMode: mount → cleanup → mount. Both registrations are in flight
    // when the first cleanup runs, which is precisely the case a bare
    // `unlisten?.()` misses.
    const onDrop = vi.fn();
    renderHook({ onDrop }, { strict: true });
    expect(registrations).toHaveLength(2);

    await act(async () => {
      registrations[0].resolve();
      registrations[1].resolve();
    });

    // The first mount's listener was cancelled before its promise settled, so
    // it must have been unlistened on arrival rather than left running.
    expect(registrations[0].unlisten).toHaveBeenCalledTimes(1);
    expect(registrations[1].unlisten).not.toHaveBeenCalled();

    // Only the surviving listener is wired to the callback: a drop delivered
    // to both would upload twice, which is the bug this guards.
    await act(async () => {
      registrations[1].listener({ payload: { type: "drop", paths: ["/a.m4a"] } });
    });
    expect(onDrop).toHaveBeenCalledTimes(1);
  });

  it("unlistens on unmount once registration has completed", async () => {
    const { view } = renderHook({ onDrop: vi.fn() });
    await act(async () => registrations[0].resolve());

    view.unmount();

    expect(registrations[0].unlisten).toHaveBeenCalledTimes(1);
  });

  it("registers once and keeps reading the latest callback", async () => {
    // Re-registering on every render would reopen the double-listener race,
    // so the hook must see new callbacks without touching the subscription.
    const first = vi.fn();
    const second = vi.fn();
    function Probe({ onDrop }: { onDrop: (paths: string[]) => void }) {
      useWebviewFileDrop({ onDrop });
      return null;
    }
    const view = render(createElement(Probe, { onDrop: first }));
    await act(async () => registrations[0].resolve());

    view.rerender(createElement(Probe, { onDrop: second }));
    expect(registrations).toHaveLength(1);

    await act(async () => {
      registrations[0].listener({ payload: { type: "drop", paths: ["/a.m4a"] } });
    });

    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith(["/a.m4a"]);
  });
});
