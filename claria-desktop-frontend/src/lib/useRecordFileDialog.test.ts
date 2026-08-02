import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useRecordFileDialog } from "./useRecordFileDialog";

const mocks = vi.hoisted(() => ({
  create: vi.fn(),
  getText: vi.fn(),
  update: vi.fn(),
}));

vi.mock("./tauri", () => ({
  createTextRecordFile: mocks.create,
  getRecordFileText: mocks.getText,
  updateTextRecordFile: mocks.update,
}));

function renderDialogHook(
  options: {
    onError?: (message: string | null) => void;
    onChanged?: () => Promise<void>;
    remove?: (filename: string) => Promise<void>;
  } = {}
) {
  const onError =
    options.onError ?? vi.fn<(message: string | null) => void>();
  const onChanged =
    options.onChanged ?? vi.fn<() => Promise<void>>().mockResolvedValue();
  const remove =
    options.remove ??
    vi.fn<(filename: string) => Promise<void>>().mockResolvedValue();
  const hook = renderHook(() =>
    useRecordFileDialog({
      clientId: "client-1",
      onError,
      onChanged,
      remove,
    })
  );
  return {
    current: () => hook.result.current,
    onError,
    onChanged,
    remove,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useRecordFileDialog", () => {
  it("keeps preview failures inside the preview dialog", async () => {
    mocks.getText.mockRejectedValue(new Error("preview unavailable"));
    const hook = renderDialogHook();

    await act(async () => hook.current().openPreview("scan.pdf"));

    expect(hook.current().dialog).toEqual({
      kind: "preview",
      filename: "scan.pdf",
      text: "Error loading preview: Error: preview unavailable",
    });
    expect(hook.onError).not.toHaveBeenCalled();
  });

  it("keeps an editor open after a failed save and closes it after success", async () => {
    mocks.getText.mockResolvedValue("original");
    mocks.update.mockRejectedValueOnce(new Error("write failed"));
    const hook = renderDialogHook();

    await act(async () => hook.current().openEdit("notes.txt"));
    await act(async () => hook.current().saveEdit("changed"));
    expect(hook.current().dialog).toEqual({
      kind: "edit",
      filename: "notes.txt",
      text: "original",
    });
    expect(hook.onError).toHaveBeenLastCalledWith("Error: write failed");

    mocks.update.mockResolvedValue(undefined);
    await act(async () => hook.current().saveEdit("changed"));
    expect(mocks.update).toHaveBeenLastCalledWith(
      "client-1",
      "notes.txt",
      "changed"
    );
    expect(hook.current().dialog).toBeNull();
    expect(hook.onChanged).toHaveBeenCalledOnce();
  });

  it("routes a drop only into an open transcription dialog", () => {
    const hook = renderDialogHook();

    expect(hook.current().divertDrop(["/tmp/voice.m4a"])).toBe(false);
    act(() => hook.current().openTranscription());
    act(() => {
      expect(hook.current().divertDrop(["/tmp/voice.m4a"])).toBe(true);
    });
    expect(hook.current().dialog).toEqual({
      kind: "transcribe",
      droppedFilePath: "/tmp/voice.m4a",
    });
  });

  it("closes a delete confirmation before delegating the delete", async () => {
    const hook = renderDialogHook();
    act(() => hook.current().openDelete("old.txt"));

    await act(async () => hook.current().confirmDelete());

    expect(hook.current().dialog).toBeNull();
    expect(hook.remove).toHaveBeenCalledWith("old.txt");
  });
});
