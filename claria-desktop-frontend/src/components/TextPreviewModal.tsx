import Modal from "./Modal";

/** Read-only view of a file's extracted text. */
export default function TextPreviewModal({
  filename,
  text,
  onClose,
}: {
  filename: string;
  text: string;
  onClose: () => void;
}) {
  return (
    <Modal
      open
      onClose={onClose}
      title={filename}
      className="max-w-2xl p-6 max-h-[80vh] flex flex-col"
    >
      <div className="flex-1 overflow-y-auto border border-gray-200 rounded-lg p-4">
        <pre className="text-sm text-gray-700 whitespace-pre-wrap font-mono">
          {text}
        </pre>
      </div>
      <div className="flex justify-end mt-4">
        <button
          onClick={onClose}
          className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
        >
          Close
        </button>
      </div>
    </Modal>
  );
}
