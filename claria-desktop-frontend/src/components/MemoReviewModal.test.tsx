import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import MemoReviewModal from "./MemoReviewModal";

/**
 * The one property of this modal that must never regress: it cannot be
 * dismissed by accident. Until Save completes, the textarea holds the only
 * copy of a recording that cannot be made again.
 */

function Harness({
  onDiscard = vi.fn(),
  onSave = vi.fn(),
  saving = false,
  initialFilename = "memo-20260101-0900",
  initialTranscript = "Session notes.",
}: {
  onDiscard?: () => void;
  onSave?: () => void;
  saving?: boolean;
  initialFilename?: string;
  initialTranscript?: string;
}) {
  const [filename, setFilename] = useState(initialFilename);
  const [transcript, setTranscript] = useState(initialTranscript);
  return (
    <MemoReviewModal
      filename={filename}
      onFilenameChange={setFilename}
      transcript={transcript}
      onTranscriptChange={setTranscript}
      saving={saving}
      onDiscard={onDiscard}
      onSave={onSave}
    />
  );
}

const dialog = () => screen.getByRole("dialog") as HTMLDialogElement;
const button = (name: string) =>
  screen.getByRole<HTMLButtonElement>("button", { name });

describe("MemoReviewModal", () => {
  it("does not close on Escape", async () => {
    const onDiscard = vi.fn();
    render(<Harness onDiscard={onDiscard} />);

    await userEvent.keyboard("{Escape}");

    expect(onDiscard).not.toHaveBeenCalled();
    expect(dialog().open).toBe(true);
  });

  it("does not close on a click outside the card", () => {
    const onDiscard = vi.fn();
    render(<Harness onDiscard={onDiscard} />);

    fireEvent.click(dialog());

    expect(onDiscard).not.toHaveBeenCalled();
  });

  it("offers no close affordance in the heading", () => {
    render(<Harness />);
    expect(screen.queryByRole("button", { name: /close/i })).toBeNull();
  });

  it("discards only through the explicit button", async () => {
    const onDiscard = vi.fn();
    render(<Harness onDiscard={onDiscard} />);

    await userEvent.click(screen.getByRole("button", { name: "Discard" }));

    expect(onDiscard).toHaveBeenCalledTimes(1);
  });

  it("keeps the transcript editable and reports edits", async () => {
    render(<Harness initialTranscript="" />);
    const textarea = document.querySelector("textarea");
    if (!textarea) throw new Error("no transcript textarea");

    await userEvent.type(textarea, "corrected");

    expect(textarea.value).toBe("corrected");
  });

  it("refuses to save without a filename", async () => {
    const onSave = vi.fn();
    render(<Harness initialFilename="   " onSave={onSave} />);

    const save = screen.getByRole<HTMLButtonElement>("button", { name: "Save" });
    expect(save.disabled).toBe(true);
    await userEvent.click(save);
    expect(onSave).not.toHaveBeenCalled();
  });

  it("disables both buttons while saving", () => {
    render(<Harness saving />);

    expect(button("Discard").disabled).toBe(true);
    expect(button("Saving...").disabled).toBe(true);
  });

  it("saves with the current filename and transcript", async () => {
    const onSave = vi.fn();
    render(<Harness onSave={onSave} />);

    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(onSave).toHaveBeenCalledTimes(1);
  });
});
