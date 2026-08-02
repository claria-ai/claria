import { useState } from "react";
import Modal from "./Modal";

/**
 * Compose a new text file directly in the record.
 *
 * The draft lives here rather than in the page, so cancelling is an unmount
 * and there is nothing to reset by hand. `onCreate` is expected to report its
 * own failures and leave the modal mounted, which keeps the user's draft on
 * screen when the write fails.
 */
export default function CreateTextFileModal({
  onCancel,
  onCreate,
}: {
  onCancel: () => void;
  onCreate: (filename: string, content: string) => Promise<void>;
}) {
  const [filename, setFilename] = useState("");
  const [content, setContent] = useState("");
  const [creating, setCreating] = useState(false);

  async function handleCreate() {
    if (!filename.trim()) return;
    setCreating(true);
    try {
      await onCreate(filename.trim(), content);
    } finally {
      setCreating(false);
    }
  }

  return (
    <Modal
      open
      onClose={onCancel}
      title="Create Text File"
      className="max-w-2xl p-6 max-h-[80vh] flex flex-col"
      showClose={false}
      dismissible={!creating}
    >
      <input
        type="text"
        placeholder="Filename (e.g. intake-notes)"
        value={filename}
        onChange={(e) => setFilename(e.target.value)}
        className="w-full px-3 py-2 border border-gray-300 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-green-500 focus:border-transparent mb-3"
        autoFocus
      />
      <textarea
        placeholder="File content..."
        value={content}
        onChange={(e) => setContent(e.target.value)}
        className="flex-1 min-h-[200px] w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono resize-none focus:outline-none focus:ring-2 focus:ring-green-500 focus:border-transparent mb-4"
      />
      <div className="flex justify-end gap-3">
        <button
          onClick={onCancel}
          className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
          disabled={creating}
        >
          Cancel
        </button>
        <button
          onClick={handleCreate}
          disabled={creating || !filename.trim()}
          className="px-4 py-2 text-sm text-white bg-green-600 rounded-lg hover:bg-green-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {creating ? "Creating..." : "Create"}
        </button>
      </div>
    </Modal>
  );
}
