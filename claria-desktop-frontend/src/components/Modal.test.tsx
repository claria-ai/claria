import { useState } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import Modal from "./Modal";

/**
 * Round-trip behaviour of the one modal shell.
 *
 * What is real here and what is not is worth being precise about, because
 * `<dialog>` is only partly implemented outside a browser. happy-dom models
 * `showModal()`/`close()` as `open`-attribute bookkeeping and dispatches the
 * `close` event, so open/closed state and the effect that drives it are
 * exercised for real. The one piece it does not model — Escape producing a
 * `cancel` event — is supplied by the spec-faithful shim in `src/test/setup.ts`.
 *
 * Not covered here, because a stub would prove nothing: the top layer,
 * focus containment, `inert` on the page behind, and `::backdrop` painting.
 * Those are browser behaviours and belong in `e2e/modal.spec.ts`.
 */

/** A modal whose `open` flag lives in React state, as every call site does. */
function Harness({
  onCloseSpy,
  dismissible = true,
  closeOnBackdropClick = false,
  initiallyOpen = true,
  unmountWhenClosed = false,
}: {
  onCloseSpy?: () => void;
  dismissible?: boolean;
  closeOnBackdropClick?: boolean;
  initiallyOpen?: boolean;
  unmountWhenClosed?: boolean;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  const close = () => {
    onCloseSpy?.();
    setOpen(false);
  };
  const modal = (
    <Modal
      open={open}
      onClose={close}
      title="Delete client?"
      dismissible={dismissible}
      closeOnBackdropClick={closeOnBackdropClick}
    >
      <p>This cannot be undone.</p>
    </Modal>
  );
  return (
    <div>
      <button onClick={() => setOpen(true)}>Reopen</button>
      {unmountWhenClosed ? open && modal : modal}
    </div>
  );
}

function dialog(): HTMLDialogElement {
  const el = document.querySelector("dialog");
  if (!el) throw new Error("no dialog in the document");
  return el as HTMLDialogElement;
}

/** The full-viewport flex container the scrim click lands on. */
function scrim(): HTMLElement {
  const el = dialog().firstElementChild;
  if (!(el instanceof HTMLElement)) throw new Error("no scrim under the dialog");
  return el;
}

describe("Escape", () => {
  it("closes the modal", async () => {
    const onClose = vi.fn();
    render(<Harness onCloseSpy={onClose} />);
    expect(dialog().open).toBe(true);

    await userEvent.keyboard("{Escape}");

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(dialog().open).toBe(false);
  });

  it("leaves the modal reopenable", async () => {
    // The footgun this guards: if the `cancel` event let the browser close the
    // dialog on its own, React would still believe `open` is true, the effect
    // would never run again, and the modal would silently refuse to reopen.
    render(<Harness />);
    await userEvent.keyboard("{Escape}");
    expect(dialog().open).toBe(false);

    await userEvent.click(screen.getByRole("button", { name: "Reopen" }));

    expect(dialog().open).toBe(true);
    expect(screen.getByText("This cannot be undone.")).toBeDefined();
  });

  it("survives several close/reopen cycles", async () => {
    render(<Harness />);
    for (let i = 0; i < 3; i++) {
      await userEvent.keyboard("{Escape}");
      expect(dialog().open).toBe(false);
      await userEvent.click(screen.getByRole("button", { name: "Reopen" }));
      expect(dialog().open).toBe(true);
    }
  });

  it("reopens a modal that unmounts while closed", async () => {
    render(<Harness unmountWhenClosed />);
    await userEvent.keyboard("{Escape}");
    expect(document.querySelector("dialog")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Reopen" }));
    expect(dialog().open).toBe(true);
  });

  it("never lets the user agent do the closing itself", () => {
    // The component's half of the contract: it must call preventDefault on
    // every cancel, so the caller's state stays the only source of truth for
    // whether the dialog is open.
    render(<Harness />);
    const cancel = new Event("cancel", { cancelable: true });
    dialog().dispatchEvent(cancel);
    expect(cancel.defaultPrevented).toBe(true);
  });
});

describe("a close that bypasses the caller", () => {
  // A `<form method="dialog">` submit, or anything else that reaches
  // `close()` directly, closes the dialog without asking React first. The
  // `close` listener is the catch-up that stops the two desyncing.
  it("catches React up", async () => {
    const onClose = vi.fn();
    render(<Harness onCloseSpy={onClose} />);

    await act(async () => dialog().close());

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("leaves the modal reopenable", async () => {
    render(<Harness />);
    await act(async () => dialog().close());

    await userEvent.click(screen.getByRole("button", { name: "Reopen" }));

    expect(dialog().open).toBe(true);
  });

  it("stays quiet when the component closed the dialog itself", async () => {
    // Closing in response to `open` going false must not bounce a second
    // onClose back at the caller.
    const onClose = vi.fn();
    render(<Harness onCloseSpy={onClose} />);
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("releases the scroll lock", async () => {
    render(<Harness />);
    await act(async () => dialog().close());
    expect(document.body.classList.contains("overflow-hidden")).toBe(false);
  });
});

describe("Escape on a modal that refuses to be dismissed", () => {
  // ClientRecord's memo-review modal is `dismissible={false}` because its
  // textarea holds the only copy of a just-recorded transcript.
  it("does not call onClose", async () => {
    const onClose = vi.fn();
    render(<Harness onCloseSpy={onClose} dismissible={false} />);

    await userEvent.keyboard("{Escape}");

    expect(onClose).not.toHaveBeenCalled();
    expect(dialog().open).toBe(true);
  });

  it("still cancels the user agent's own close", () => {
    // This, not the assertion above, is the load-bearing one: `open` staying
    // true is only meaningful because preventDefault was called.
    render(<Harness dismissible={false} />);
    const cancel = new Event("cancel", { cancelable: true });
    dialog().dispatchEvent(cancel);
    expect(cancel.defaultPrevented).toBe(true);
  });

  it("keeps its content mounted", async () => {
    render(<Harness dismissible={false} />);
    await userEvent.keyboard("{Escape}");
    expect(screen.getByText("This cannot be undone.")).toBeDefined();
  });

  it("disables the close button", () => {
    render(<Harness dismissible={false} />);
    expect(screen.getByRole("button", { name: "Close" })).toHaveProperty(
      "disabled",
      true
    );
  });
});

describe("backdrop click", () => {
  // A real browser lands this click on the full-viewport flex container: the
  // dialog is `fixed inset-0` and the container is `h-full w-full` inside it,
  // so anything outside the card hits the container and nothing else.
  it("closes a modal that opted in, as the transcribe wizard does", async () => {
    const onClose = vi.fn();
    render(<Harness onCloseSpy={onClose} closeOnBackdropClick />);

    fireEvent.click(scrim());

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(dialog().open).toBe(false);
  });

  it("does not close a delete confirmation", async () => {
    // Destructive confirmations keep the default, so a stray click cannot
    // dismiss the question.
    const onClose = vi.fn();
    render(<Harness onCloseSpy={onClose} />);

    fireEvent.click(scrim());

    expect(onClose).not.toHaveBeenCalled();
    expect(dialog().open).toBe(true);
  });

  it("does not close a modal that opted in while work is in flight", () => {
    const onClose = vi.fn();
    render(<Harness onCloseSpy={onClose} closeOnBackdropClick dismissible={false} />);

    fireEvent.click(scrim());

    expect(onClose).not.toHaveBeenCalled();
    expect(dialog().open).toBe(true);
  });

  it("ignores a click that started inside the card", async () => {
    const onClose = vi.fn();
    render(<Harness onCloseSpy={onClose} closeOnBackdropClick />);

    await userEvent.click(screen.getByText("This cannot be undone."));

    expect(onClose).not.toHaveBeenCalled();
    expect(dialog().open).toBe(true);
  });
});

describe("the X button", () => {
  it("closes the modal", async () => {
    const onClose = vi.fn();
    render(<Harness onCloseSpy={onClose} />);

    await userEvent.click(screen.getByRole("button", { name: "Close" }));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(dialog().open).toBe(false);
  });
});

describe("scroll lock", () => {
  const locked = () => document.body.classList.contains("overflow-hidden");

  it("engages while a modal is open", () => {
    expect(locked()).toBe(false);
    render(<Harness />);
    expect(locked()).toBe(true);
  });

  it("releases when the modal closes", async () => {
    render(<Harness />);
    await userEvent.keyboard("{Escape}");
    expect(locked()).toBe(false);
  });

  it("releases when the modal unmounts without closing first", () => {
    const { unmount } = render(<Harness />);
    expect(locked()).toBe(true);
    unmount();
    expect(locked()).toBe(false);
  });

  it("stays engaged while a second modal is still open", async () => {
    render(
      <div>
        <Harness />
        <Harness />
      </div>
    );
    expect(locked()).toBe(true);

    // Close the topmost one; the other is still up, so the page must not
    // start scrolling behind it.
    await userEvent.keyboard("{Escape}");
    expect(locked()).toBe(true);

    await userEvent.keyboard("{Escape}");
    expect(locked()).toBe(false);
  });

  it("does not engage for a modal rendered closed", () => {
    render(<Harness initiallyOpen={false} />);
    expect(locked()).toBe(false);
  });
});

describe("accessibility wiring", () => {
  it("names the dialog with its own heading", () => {
    render(<Harness />);
    const labelledBy = dialog().getAttribute("aria-labelledby");
    expect(labelledBy).toBeTruthy();
    expect(document.getElementById(labelledBy ?? "")?.textContent).toBe(
      "Delete client?"
    );
  });

  it("gives two modals distinct heading ids", () => {
    render(
      <div>
        <Harness />
        <Harness />
      </div>
    );
    const ids = [...document.querySelectorAll("dialog")].map((d) =>
      d.getAttribute("aria-labelledby")
    );
    expect(new Set(ids).size).toBe(2);
  });
});
