import { useEffect, useId, useRef, type ReactNode } from "react";
import { CloseIcon } from "./icons";

/**
 * The one modal shell.
 *
 * Built on the native `<dialog>` element opened with `showModal()`, which
 * gives focus containment, Escape handling and top-layer stacking for free,
 * and paints the scrim through `::backdrop` — none of which the hand-rolled
 * `fixed inset-0 bg-black/40` overlays it replaces had.
 *
 * The dialog element itself is the full-viewport overlay (transparent, with
 * `::backdrop` behind it); the white card is a child, so call sites keep
 * the card markup they already had.
 *
 * Open state is owned by the caller. The effect below is the only thing
 * that calls `showModal()`/`close()`, and the `cancel` event is cancelled so
 * that Escape routes through `onClose` instead of closing the dialog behind
 * React's back — otherwise the DOM and React state desync and the modal
 * cannot be reopened.
 *
 * Both mounting patterns work:
 *   {flag && <Modal open …>}   — unmounts when closed
 *   <Modal open={flag} …>      — stays mounted
 */
export default function Modal({
  open,
  onClose,
  title,
  variant = "padded",
  className = "",
  dismissible = true,
  closeOnBackdropClick = false,
  showClose = true,
  children,
}: {
  open: boolean;
  /** Called for every user-initiated dismissal: Escape, the X, the scrim. */
  onClose: () => void;
  /** Visible heading. Also supplies the dialog's accessible name. */
  title: ReactNode;
  /**
   * `padded` puts the heading inside the card's own padding; `framed` makes
   * it a full-bleed row with a bottom border.
   */
  variant?: "padded" | "framed";
  /** Card classes — sizing, padding and layout, e.g. `max-w-2xl p-6 flex flex-col`. */
  className?: string;
  /**
   * Whether the user may dismiss the dialog right now. False blocks Escape
   * and the scrim and disables the X — use it while work is in flight.
   */
  dismissible?: boolean;
  /** Whether clicking the scrim dismisses. Off by default, as it was in every overlay but one. */
  closeOnBackdropClick?: boolean;
  /** Render the X in the heading row. */
  showClose?: boolean;
  children: ReactNode;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  // Set while we close the dialog ourselves, so the resulting `close` event
  // is not mistaken for a dismissal the caller needs to hear about. React's
  // StrictMode remount in development closes and reopens the dialog, which
  // would otherwise fire a spurious onClose.
  const selfClosingRef = useRef(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;

    const closeSelf = () => {
      if (!dialog.open) return;
      selfClosingRef.current = true;
      dialog.close();
    };

    if (!open) {
      closeSelf();
      return;
    }

    if (!dialog.open) dialog.showModal();
    lockBodyScroll();
    return () => {
      unlockBodyScroll();
      closeSelf();
    };
  }, [open]);

  return (
    <dialog
      ref={dialogRef}
      aria-modal="true"
      aria-labelledby={titleId}
      onCancel={(e) => {
        // Escape. Never let the browser close the dialog on its own — the
        // caller's state is the single source of truth for `open`.
        e.preventDefault();
        if (dismissible) onClose();
      }}
      onClose={() => {
        if (selfClosingRef.current) {
          selfClosingRef.current = false;
          return;
        }
        // Something closed the dialog without going through the caller
        // (e.g. a form submitted with method="dialog"). Catch React up.
        if (open) onClose();
      }}
      className={DIALOG_CLASSES}
    >
      <div
        className="flex h-full w-full items-center justify-center p-4"
        onClick={(e) => {
          if (e.target !== e.currentTarget) return;
          if (dismissible && closeOnBackdropClick) onClose();
        }}
      >
        <div className={`w-full bg-white rounded-xl shadow-lg ${className}`}>
          <div
            className={
              variant === "framed"
                ? "px-5 py-4 border-b border-gray-200 flex items-center justify-between"
                : "flex items-center justify-between mb-4"
            }
          >
            <h3
              id={titleId}
              className={
                variant === "framed"
                  ? "font-semibold text-gray-900"
                  : "text-lg font-semibold text-gray-900"
              }
            >
              {title}
            </h3>
            {showClose && (
              <button
                onClick={onClose}
                disabled={!dismissible}
                aria-label="Close"
                className="text-gray-400 hover:text-gray-600 transition-colors disabled:opacity-50"
              >
                <CloseIcon />
              </button>
            )}
          </div>
          {children}
        </div>
      </div>
    </dialog>
  );
}

/**
 * The dialog element is the overlay, not the card: full viewport, no
 * chrome of its own, scrim painted by `::backdrop`.
 *
 * `max-h-none`/`max-w-none` undo the user-agent's `calc(100% - 6px - 2em)`
 * caps on modal dialogs, and `text-inherit` undoes its `color: CanvasText`
 * — without that, anything inside a modal that relies on inherited colour
 * rather than an explicit `text-*` class gets repainted pure black.
 */
const DIALOG_CLASSES =
  "fixed inset-0 h-full max-h-none w-full max-w-none overflow-hidden bg-transparent text-inherit backdrop:bg-black/40";

// `showModal()` leaves the page behind the dialog scrollable, so hold the
// body still for as long as at least one modal is open.
let openModals = 0;

function lockBodyScroll() {
  if (openModals++ === 0) document.body.classList.add("overflow-hidden");
}

function unlockBodyScroll() {
  if (--openModals === 0) document.body.classList.remove("overflow-hidden");
}
