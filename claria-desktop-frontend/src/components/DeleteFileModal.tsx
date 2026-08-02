import Modal from "./Modal";

/**
 * Confirm deleting a file and its extracted text.
 *
 * Phrased as recoverable because it is: the bucket is versioned and the
 * record's More mode restores deleted files.
 */
export default function DeleteFileModal({
  filename,
  onCancel,
  onConfirm,
}: {
  filename: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Modal
      open
      onClose={onCancel}
      title="Delete file?"
      className="max-w-sm p-6"
      showClose={false}
    >
      <p className="text-sm text-gray-600 mb-6">
        Delete <span className="font-medium">{filename}</span> and its extracted
        text? You can restore this file again later if necessary.
      </p>
      <div className="flex justify-end gap-3">
        <button
          onClick={onCancel}
          className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
        >
          Cancel
        </button>
        <button
          onClick={onConfirm}
          className="px-4 py-2 text-sm text-white bg-red-600 rounded-lg hover:bg-red-700 transition-colors"
        >
          Delete
        </button>
      </div>
    </Modal>
  );
}
