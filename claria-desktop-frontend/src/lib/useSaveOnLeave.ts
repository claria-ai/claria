import { useCallback, useEffect, useRef, type FocusEvent } from "react";

/**
 * Save-on-leave for an auto-saving preferences section: instead of one write
 * per control change — which turned every radio click into an S3 version of
 * the preferences file — edits accumulate in the section's draft and `flush`
 * runs when the user leaves the section or the screen goes away:
 *
 * - a pointer press anywhere outside the section's container (macOS WebKit
 *   does not focus radios or buttons on click, so focus events alone never
 *   fire for the mouse flow),
 * - focus moving outside the container (the keyboard flow),
 * - the section unmounting (navigating away from Preferences),
 * - `pagehide` (window closing — best effort: the invoke may not complete
 *   if the WebView tears down first).
 *
 * `flush` must no-op when there is nothing to save; leave events fire
 * regardless of dirtiness. The latest `flush` is always used, so callers
 * don't need to memoize it.
 */
export function useSaveOnLeave(flush: () => void): {
  /** Attach to the section's content container. */
  containerRef: (node: HTMLDivElement | null) => void;
  /** Attach as `onBlur` on the same container (blur bubbles in React). */
  onContainerBlur: (event: FocusEvent<HTMLElement>) => void;
} {
  const nodeRef = useRef<HTMLDivElement | null>(null);
  const flushRef = useRef(flush);
  useEffect(() => {
    flushRef.current = flush;
  });

  useEffect(() => {
    // Capture phase, so the save starts before a click's own handler can
    // unmount this section (e.g. the Back button).
    const onPointerDown = (event: PointerEvent) => {
      const container = nodeRef.current;
      if (
        container &&
        event.target instanceof Node &&
        !container.contains(event.target)
      ) {
        flushRef.current();
      }
    };
    const onPageHide = () => flushRef.current();
    document.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("pagehide", onPageHide);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("pagehide", onPageHide);
      flushRef.current();
    };
  }, []);

  const containerRef = useCallback((node: HTMLDivElement | null) => {
    nodeRef.current = node;
  }, []);

  const onContainerBlur = useCallback((event: FocusEvent<HTMLElement>) => {
    const next = event.relatedTarget;
    if (!(next instanceof Node) || !event.currentTarget.contains(next)) {
      flushRef.current();
    }
  }, []);

  return { containerRef, onContainerBlur };
}
