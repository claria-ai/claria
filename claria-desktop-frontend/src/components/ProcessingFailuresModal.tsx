import { useRef } from "react";
import type { ProcessingFile } from "../lib/fileProcessing";
import Modal from "./Modal";
import FileIcon from "./FileIcon";

export default function ProcessingFailuresModal({ files, retrying, onRetry, onChooseModel, onClose }: {
  files: ProcessingFile[];
  retrying: string | null;
  onRetry: (filename: string) => Promise<void>;
  /** Open Preferences at the extraction model; a retry cannot fix a missing model. */
  onChooseModel: () => void;
  onClose: () => void;
}) {
  const done = useRef<HTMLButtonElement>(null);
  async function retry(filename: string) {
    await onRetry(filename);
    // Retain a predictable focus target when the completed row disappears.
    done.current?.focus();
  }
  const failed = files.filter((file) => file.processing_status?.status === "failed").length;
  const unknownOnly = files.length > 0 && failed === 0;
  return <Modal open title={files.length === 0 ? "Text processing complete" : unknownOnly ? "Text is not available" : "Files need attention"}
    onClose={onClose} className="max-w-2xl p-6">
    <p className="text-sm text-gray-600 mb-4" role="status">{files.length === 0
      ? "Text is ready to use in previews, Chat, and Writing."
      : unknownOnly ? "The original is saved, but it has no extracted text yet."
      : `${failed} ${failed === 1 ? "file" : "files"} failed to process. Review the reasons below.`}</p>
    <div className="divide-y divide-gray-100 max-h-[55vh] overflow-y-auto">
      {files.map((file) => {
        const status = file.processing_status;
        const needsModel = status?.status === "failed" && status.code === "model_not_configured";
        return <div key={file.filename} className="py-4 flex gap-3 items-start">
          <FileIcon filename={file.filename} />
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium text-gray-900 break-words">{file.filename}</p>
            <p className="text-xs text-gray-500 mt-1">{file.originalSaved === false ? "Original upload was not confirmed." : "Original saved. Text processing did not finish."}</p>
            <p className="text-sm text-amber-800 mt-2">{status?.status === "failed"
              ? status.reason : "No extracted text is available. Extract it to use this file in Chat and Writing."}</p>
          </div>
          {file.originalSaved !== false && (needsModel
            ? <button onClick={onChooseModel}
                className="px-3 py-1.5 text-sm text-blue-600 hover:bg-blue-50 rounded-lg">
                Choose model
              </button>
            : <button disabled={retrying !== null} onClick={() => void retry(file.filename)}
                className="px-3 py-1.5 text-sm text-blue-600 hover:bg-blue-50 rounded-lg disabled:opacity-50">
                {retrying === file.filename ? "Processing…" : status?.status === "unknown" ? "Extract text" : "Retry processing"}
              </button>)}
        </div>;
      })}
    </div>
    <p className="text-xs text-gray-500 mt-4">Processing retries may incur AWS charges. Closing this window does not delete your files.</p>
    <div className="flex justify-end mt-5">
      <button ref={done} onClick={onClose} className="px-4 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700">Done</button>
    </div>
  </Modal>;
}
