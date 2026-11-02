import { useRef } from "react";
import type { ProcessingFile } from "../lib/fileProcessing";
import Modal from "./Modal";
import FileIcon from "./FileIcon";

export default function ProcessingFailuresModal({ files, retrying, onRetry, onClose }: {
  files: ProcessingFile[]; retrying: string | null; onRetry: (filename: string) => Promise<void>; onClose: () => void;
}) {
  const done = useRef<HTMLButtonElement>(null);
  async function retry(filename: string) {
    await onRetry(filename);
    // Retain a predictable focus target when the completed row disappears.
    done.current?.focus();
  }
  const unknown = files.length === 1 && files[0].processing_status?.status === "unknown";
  return <Modal open title={files.length === 0 ? "Text processing complete" : unknown ? "Text is not available" : "Files need attention"}
    onClose={onClose} className="max-w-2xl p-6">
    <p className="text-sm text-gray-600 mb-4" role="status">{files.length === 0
      ? "Text is ready to use in previews, Chat, and Writing."
      : unknown ? "The original is saved. Its previous processing outcome is unknown."
      : `${files.length} ${files.length === 1 ? "file failed" : "files failed"} to process. Review the reasons below.`}</p>
    <div className="divide-y divide-gray-100 max-h-[55vh] overflow-y-auto">
      {files.map((file) => <div key={file.filename} className="py-4 flex gap-3 items-start">
        <FileIcon filename={file.filename} />
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium text-gray-900 break-words">{file.filename}</p>
          <p className="text-xs text-gray-500 mt-1">{file.originalSaved === false ? "Original upload was not confirmed." : "Original saved. Text processing did not finish."}</p>
          <p className="text-sm text-amber-800 mt-2">{file.processing_status?.status === "failed"
            ? file.processing_status.reason : "No usable extracted text is available. Retry to process this file."}</p>
        </div>
        {file.originalSaved !== false && <button disabled={retrying !== null} onClick={() => void retry(file.filename)}
          className="px-3 py-1.5 text-sm text-blue-600 hover:bg-blue-50 rounded-lg disabled:opacity-50">
          {retrying === file.filename ? "Processing…" : file.processing_status?.status === "unknown" ? "Extract text" : "Retry processing"}
        </button>}
      </div>)}
    </div>
    <p className="text-xs text-gray-500 mt-4">Processing retries may incur AWS charges. Closing this window does not delete your files.</p>
    <div className="flex justify-end mt-5">
      <button ref={done} onClick={onClose} className="px-4 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700">Done</button>
    </div>
  </Modal>;
}
