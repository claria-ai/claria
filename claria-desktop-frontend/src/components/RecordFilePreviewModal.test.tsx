import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import RecordFilePreviewModal from "./RecordFilePreviewModal";

const mocks = vi.hoisted(() => ({ getText: vi.fn() }));

vi.mock("../lib/tauri", () => ({
  getRecordFileText: mocks.getText,
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("RecordFilePreviewModal", () => {
  it("shows context text already loaded by Chat without fetching it again", () => {
    render(
      <RecordFilePreviewModal
        filename="intake.txt"
        text="Already loaded context"
        onClose={vi.fn()}
      />
    );

    expect(screen.getByText("Already loaded context")).toBeDefined();
    expect(mocks.getText).not.toHaveBeenCalled();
  });

  it("loads structured text through the shared record preview path", async () => {
    mocks.getText.mockResolvedValue('{"score": 12}');

    render(
      <RecordFilePreviewModal
        clientId="client-1"
        filename="scores.json"
        onClose={vi.fn()}
      />
    );

    expect(screen.getByRole("status").textContent).toContain("Loading preview");
    expect(await screen.findByText('{"score": 12}')).toBeDefined();
    expect(mocks.getText).toHaveBeenCalledWith("client-1", "scores.json");
  });

  it("keeps preview failures inside the preview window", async () => {
    mocks.getText.mockRejectedValue(new Error("preview unavailable"));

    render(
      <RecordFilePreviewModal
        clientId="client-1"
        filename="scan.pdf"
        onClose={vi.fn()}
      />
    );

    expect(
      await screen.findByText("Error loading preview: Error: preview unavailable")
    ).toBeDefined();
  });
});
