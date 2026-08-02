import { useState } from "react";
import Modal from "./Modal";

/**
 * Edit an existing text file in place.
 *
 * Mounted only once the current contents have been fetched, so the textarea
 * starts from real text rather than an empty box the user could save over.
 */
export default function EditTextModal({
  filename,
  initialText,
  onCancel,
  onSave,
}: {
  filename: string;
  initialText: string;
  onCancel: () => void;
  onSave: (text: string) => Promise<void>;
}) {
  const [text, setText] = useState(initialText);
  const [saving, setSaving] = useState(false);

  async function handleSave() {
    setSaving(true);
    try {
      await onSave(text);
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal
      open
      onClose={onCancel}
      title={filename}
      className="max-w-2xl p-6 max-h-[80vh] flex flex-col"
      showClose={false}
      dismissible={!saving}
    >
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        disabled={saving}
        className="flex-1 min-h-[300px] w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono resize-none focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50"
      />
      <div className="flex justify-end gap-3 mt-4">
        <button
          onClick={onCancel}
          disabled={saving}
          className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800 disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          onClick={handleSave}
          disabled={saving}
          className="px-4 py-2 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
        >
          {saving ? "Saving..." : "Save"}
        </button>
      </div>
    </Modal>
  );
}
