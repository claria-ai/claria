import CreateTextFileModal from "./CreateTextFileModal";
import DeleteFileModal from "./DeleteFileModal";
import EditTextModal from "./EditTextModal";
import RecordFilePreviewModal from "./RecordFilePreviewModal";
import TranscribeWizard from "./TranscribeWizard";
import VersionHistoryModal from "./VersionHistoryModal";
import { isAudioSidecar } from "../lib/recordFiles";
import { recordFileVersions } from "../lib/versions";
import { type RecordFileDialogController } from "../lib/useRecordFileDialog";

/** Render the one file dialog selected by the Record workspace. */
export default function RecordFileDialogs({
  clientId,
  dragging,
  controller,
  onError,
}: {
  clientId: string;
  dragging: boolean;
  controller: RecordFileDialogController;
  onError: (message: string) => void;
}) {
  const { dialog } = controller;
  if (!dialog) return null;

  switch (dialog.kind) {
    case "preview":
      return (
        <RecordFilePreviewModal
          clientId={clientId}
          filename={dialog.filename}
          onClose={controller.close}
        />
      );
    case "edit":
      return (
        <EditTextModal
          filename={dialog.filename}
          initialText={dialog.text}
          onCancel={controller.close}
          onSave={controller.saveEdit}
        />
      );
    case "delete":
      return (
        <DeleteFileModal
          filename={dialog.filename}
          onCancel={controller.close}
          onConfirm={() => void controller.confirmDelete()}
        />
      );
    case "create":
      return (
        <CreateTextFileModal
          onCancel={controller.close}
          onCreate={controller.create}
        />
      );
    case "transcribe":
      return (
        <TranscribeWizard
          clientId={clientId}
          droppedFilePath={dialog.droppedFilePath}
          isDragging={dragging}
          onClose={controller.close}
          onUploaded={() => void controller.refresh()}
        />
      );
    case "versions":
      return (
        <VersionHistoryModal
          title={
            isAudioSidecar(dialog.filename)
              ? `Transcript History: ${dialog.filename}`
              : `Version History: ${dialog.filename}`
          }
          source={recordFileVersions(clientId, dialog.filename)}
          onClose={controller.close}
          onRestored={controller.refresh}
          onError={onError}
          enableCompare
          showFooterClose
          className="max-w-4xl p-6 max-h-[90vh] flex flex-col"
        />
      );
  }
}
