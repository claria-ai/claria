import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => {
  cleanup();
  document.body.className = "";
});

// ---------------------------------------------------------------------------
// The one user-agent behaviour happy-dom is missing: canceling a dialog
// ---------------------------------------------------------------------------
//
// happy-dom models `<dialog>` as attribute bookkeeping — `showModal()` sets
// `open`, `close()` clears it and fires `close`. That is enough to observe
// open/closed state honestly, but it never turns an Escape keypress into the
// `cancel` event that `Modal.tsx` is written against, so without this the
// Escape path would be untestable.
//
// Installed here is exactly that step of the HTML spec's "canceling dialogs"
// algorithm and nothing else: Escape on the top open dialog fires a cancelable
// `cancel` event, and the dialog closes only if no listener called
// `preventDefault()`.
//
//   https://html.spec.whatwg.org/multipage/interactive-elements.html#canceling-dialogs
//
// `Modal.tsx` always calls `preventDefault()`, so the close branch below never
// runs for our modals — the shim's only real contribution is synthesizing the
// event. Everything downstream of it (routing through `onClose`, React driving
// `showModal()`/`close()`) is the component's own code running for real.
//
// Deliberately NOT modelled, because a stub would prove nothing: the top
// layer, focus containment, `inert` on the rest of the page, and `::backdrop`
// rendering. Those are browser-only and are asserted in `e2e/modal.spec.ts`.
document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  const open = document.querySelectorAll<HTMLDialogElement>("dialog[open]");
  const topmost = open[open.length - 1];
  if (!topmost) return;
  const cancel = new Event("cancel", { cancelable: true });
  topmost.dispatchEvent(cancel);
  if (!cancel.defaultPrevented) topmost.close();
});
