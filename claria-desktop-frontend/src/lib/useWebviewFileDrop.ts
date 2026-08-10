import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { logFrontendEvent } from "./logBridge";

/**
 * Webview-level file drag-and-drop, reported as a `dragging` flag plus a drop
 * callback.
 *
 * Two subtleties live here, and both are the reason this is a hook rather
 * than an effect copied into a component:
 *
 * 1. **StrictMode race.** `onDragDropEvent` returns a Promise that resolves
 *    to the unlisten function. In React StrictMode, effects mount → clean
 *    up → mount again. The first cleanup runs before the Promise resolves,
 *    so a bare `unlisten?.()` is a no-op and both mounts end up with live
 *    listeners — the bug the user observed as "two uploads at once". The
 *    `cancelled` flag drains the second listener if cleanup runs before
 *    registration completes.
 *
 * 2. **Modal stacking.** Tauri's drag-drop is webview-level, not DOM-level,
 *    so a drop onto an open modal still reaches this listener and would run
 *    the page's default drop handling underneath it. `divert` gets first
 *    refusal on every drop and returning `true` consumes it.
 *
 * Registration happens once per mount. The callbacks are read through refs so
 * that a re-render never re-registers the listener and never leaves the
 * handler looking at a stale closure.
 */
export function useWebviewFileDrop({
  onDrop,
  divert,
}: {
  onDrop: (paths: string[]) => void;
  /** First refusal on a drop. Return `true` to consume it. */
  divert?: (paths: string[]) => boolean;
}): boolean {
  const [dragging, setDragging] = useState(false);

  const onDropRef = useRef(onDrop);
  const divertRef = useRef(divert);
  useEffect(() => {
    onDropRef.current = onDrop;
    divertRef.current = divert;
  });

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setDragging(true);
        } else if (event.payload.type === "leave") {
          setDragging(false);
        } else if (event.payload.type === "drop") {
          setDragging(false);
          if (divertRef.current?.(event.payload.paths)) return;
          onDropRef.current(event.payload.paths);
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err) => {
        logFrontendEvent(
          "error",
          `Failed to register drag-drop listener: ${err}`
        );
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return dragging;
}
