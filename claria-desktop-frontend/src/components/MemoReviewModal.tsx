import Modal from "./Modal";

/**
 * Name-and-save step for a just-finished memo.
 *
 * Not dismissible, and deliberately so: the only copy of the transcript lives
 * in this form until Save writes it to S3, and Escape or a scrim click would
 * throw away a whole session's recording with no way to get it back short of
 * recording it again. Discard is an explicit button.
 */
export default function MemoReviewModal({
  filename,
  onFilenameChange,
  transcript,
  onTranscriptChange,
  saving,
  onDiscard,
  onSave,
}: {
  filename: string;
  onFilenameChange: (filename: string) => void;
  transcript: string;
  onTranscriptChange: (transcript: string) => void;
  saving: boolean;
  onDiscard: () => void;
  onSave: () => void;
}) {
  return (
    <Modal
      open
      onClose={onDiscard}
      title="Review Memo"
      className="max-w-2xl p-6 max-h-[80vh] flex flex-col"
      showClose={false}
      dismissible={false}
    >
      <input
        type="text"
        placeholder="Filename (e.g. session-notes)"
        value={filename}
        onChange={(e) => onFilenameChange(e.target.value)}
        className="w-full px-3 py-2 border border-gray-300 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent mb-3"
        autoFocus
      />
      <textarea
        value={transcript}
        onChange={(e) => onTranscriptChange(e.target.value)}
        className="flex-1 min-h-[200px] w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono resize-none focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent mb-4"
      />
      <div className="flex justify-end gap-3">
        <button
          onClick={onDiscard}
          className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
          disabled={saving}
        >
          Discard
        </button>
        <button
          onClick={onSave}
          disabled={saving || !filename.trim()}
          className="px-4 py-2 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {saving ? "Saving..." : "Save"}
        </button>
      </div>
    </Modal>
  );
}
