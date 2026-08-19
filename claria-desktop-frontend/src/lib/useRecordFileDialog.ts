import { useState } from "react";
import {
  createTextRecordFile,
  getRecordFileText,
  updateTextRecordFile,
} from "./tauri";

export type RecordFileDialog =
  | { kind: "preview"; filename: string }
  | { kind: "edit"; filename: string; text: string }
  | { kind: "delete"; filename: string }
  | { kind: "create" }
  | { kind: "transcribe"; droppedFilePath: string | null }
  | { kind: "versions"; filename: string }
  | null;

/**
 * The mutually exclusive dialogs launched from the Record file panel.
 *
 * Fetching and writes live with the dialog state so the page only decides
 * which file action the user requested. Failed edits and creates leave their
 * dialog mounted, preserving the user's draft while the shared error banner
 * explains the failure.
 */
export function useRecordFileDialog({
  clientId,
  onError,
  onChanged,
  remove,
}: {
  clientId: string;
  onError: (message: string | null) => void;
  onChanged: () => Promise<void>;
  remove: (filename: string) => Promise<void>;
}) {
  const [dialog, setDialog] = useState<RecordFileDialog>(null);

  function openPreview(filename: string) {
    setDialog({ kind: "preview", filename });
  }

  async function openEdit(filename: string) {
    try {
      setDialog({
        kind: "edit",
        filename,
        text: await getRecordFileText(clientId, filename),
      });
    } catch (error) {
      onError(String(error));
    }
  }

  async function saveEdit(text: string) {
    if (dialog?.kind !== "edit") return;
    onError(null);
    try {
      await updateTextRecordFile(clientId, dialog.filename, text);
      setDialog(null);
      await onChanged();
    } catch (error) {
      onError(String(error));
    }
  }

  async function confirmDelete() {
    if (dialog?.kind !== "delete") return;
    const { filename } = dialog;
    setDialog(null);
    await remove(filename);
  }

  async function create(filename: string, content: string) {
    onError(null);
    try {
      await createTextRecordFile(clientId, filename, content);
      setDialog(null);
      await onChanged();
    } catch (error) {
      onError(String(error));
    }
  }

  function divertDrop(paths: string[]): boolean {
    if (dialog?.kind !== "transcribe") return false;
    const first = paths[0];
    if (first) {
      setDialog({ kind: "transcribe", droppedFilePath: first });
    }
    return true;
  }

  return {
    dialog,
    close: () => setDialog(null),
    openPreview,
    openEdit,
    openDelete: (filename: string) =>
      setDialog({ kind: "delete", filename }),
    openCreate: () => setDialog({ kind: "create" }),
    openTranscription: () =>
      setDialog({ kind: "transcribe", droppedFilePath: null }),
    openVersions: (filename: string) =>
      setDialog({ kind: "versions", filename }),
    saveEdit,
    confirmDelete,
    create,
    divertDrop,
    refresh: onChanged,
  };
}

export type RecordFileDialogController = ReturnType<
  typeof useRecordFileDialog
>;
