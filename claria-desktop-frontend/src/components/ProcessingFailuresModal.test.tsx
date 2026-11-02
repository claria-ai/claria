import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import ProcessingFailuresModal from "./ProcessingFailuresModal";
it("offers retry only when the original was saved and preserves a completion focus target", async () => {
  const retry = vi.fn().mockResolvedValue(undefined);
  const files = [{ filename: "saved.pdf", size: 1, uploaded_at: null, processing_status: { status: "failed" as const, code: "invalid_text" as const, reason: "Empty response" } },
    { filename: "missing.pdf", size: 0, uploaded_at: null, originalSaved: false, processing_status: { status: "failed" as const, code: "storage_unavailable" as const, reason: "Upload not confirmed" } }];
  const view = render(<ProcessingFailuresModal files={files} retrying={null} onRetry={retry} onClose={() => {}} />);
  expect(screen.getAllByRole("button", { name: "Retry processing" })).toHaveLength(1);
  expect(screen.getByText("Original upload was not confirmed.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Retry processing" }));
  await waitFor(() => expect(retry).toHaveBeenCalledWith("saved.pdf"));
  view.rerender(<ProcessingFailuresModal files={[]} retrying={null} onRetry={retry} onClose={() => {}} />);
  expect(screen.getByText("Text processing complete")).toBeTruthy();
});
