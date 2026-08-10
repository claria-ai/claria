import Modal from "./Modal";
import { MarkdownBlock } from "./Markdown";

/** Read-only view of an uploaded text file or generated text representation. */
export default function TextPreviewModal({
  filename,
  text,
  markdown = false,
  onClose,
}: {
  filename: string;
  text: string;
  /** Render the body as Markdown instead of preformatted monospace. */
  markdown?: boolean;
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
        {markdown ? (
          <MarkdownBlock source={text} variant="plain" />
        ) : (
          <pre className="text-sm text-gray-700 whitespace-pre-wrap font-mono">
            {text}
          </pre>
        )}
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
