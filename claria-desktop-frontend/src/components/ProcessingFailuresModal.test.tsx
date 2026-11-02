import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import ProcessingFailuresModal from "./ProcessingFailuresModal";
it("offers retry only when the original was saved and preserves a completion focus target", async () => {
  const retry = vi.fn().mockResolvedValue(undefined);
  const files = [{ filename: "saved.pdf", size: 1, uploaded_at: null, processing_status: { status: "failed" as const, code: "invalid_text" as const, reason: "Empty response" } },
    { filename: "missing.pdf", size: 0, uploaded_at: null, originalSaved: false, processing_status: { status: "failed" as const, code: "storage_unavailable" as const, reason: "Upload not confirmed" } }];
  const view = render(<ProcessingFailuresModal files={files} retrying={null} onRetry={retry} onChooseModel={() => {}} onClose={() => {}} />);
  expect(screen.getAllByRole("button", { name: "Retry processing" })).toHaveLength(1);
  expect(screen.getByText("Original upload was not confirmed.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Retry processing" }));
  await waitFor(() => expect(retry).toHaveBeenCalledWith("saved.pdf"));
  view.rerender(<ProcessingFailuresModal files={[]} retrying={null} onRetry={retry} onChooseModel={() => {}} onClose={() => {}} />);
  expect(screen.getByText("Text processing complete")).toBeTruthy();
});

it("routes a missing extraction model to Preferences instead of a retry that cannot work", () => {
  const choose = vi.fn();
  const files = [{ filename: "intake.pdf", size: 1, uploaded_at: null,
    processing_status: { status: "failed" as const, code: "model_not_configured" as const, reason: "Choose a document extraction model in Preferences, then retry." } }];
  render(<ProcessingFailuresModal files={files} retrying={null} onRetry={vi.fn()} onChooseModel={choose} onClose={() => {}} />);
  expect(screen.queryByRole("button", { name: "Retry processing" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Choose model" }));
  expect(choose).toHaveBeenCalledOnce();
  expect(screen.getByText("1 file failed to process. Review the reasons below.")).toBeTruthy();
});
