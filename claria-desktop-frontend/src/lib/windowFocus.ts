import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Whether this Claria window is frontmost right now.
 *
 * Asked of Tauri rather than of the DOM. `document.hasFocus()` answers about
 * focus *within* the page and stays true in cases the window manager does not
 * consider the app frontmost, and the thing being guarded here is a
 * system-modal panel that would land on top of whatever else is on screen.
 * `isFocused()` is implemented on every desktop platform Tauri supports, so
 * macOS and Windows read the same.
 *
 * Read at the moment of use rather than tracked in state: a click queued
 * before the window went away must not act on an answer that was true a tick
 * ago.
 */
export async function isWindowFocused(): Promise<boolean> {
  try {
    return await getCurrentWindow().isFocused();
  } catch {
    // No Tauri host behind this window — a browser harness, a screenshot run.
    // There is no sensor to raise a panel with either, so the DOM's answer is
    // as good as it gets.
    return typeof document === "undefined" || document.hasFocus();
  }
}
