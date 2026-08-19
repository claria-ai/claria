import { useState } from "react";
import { getRecordFileText } from "../lib/tauri";
import { isAudioSidecar } from "../lib/recordFiles";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import Modal from "./Modal";
import Spinner from "./Spinner";
import TextPreviewModal from "./TextPreviewModal";
import TranscriptEditor from "./TranscriptEditor";

type PreviewProps = {
  filename: string;
  onClose: () => void;
} & (
  | { text: string; clientId?: never; readOnly?: true }
  | { clientId: string; text?: never; readOnly?: boolean }
);

/**
 * Shared record-context preview for Chat and Writing. Chat can supply text it
 * already loaded; Writing resolves the selected filename lazily. The Record
 * eye action uses the same loader but retains transcript editing for audio.
 */
export default function RecordFilePreviewModal(props: PreviewProps) {
  if (props.text !== undefined) {
    return (
      <TextPreviewModal
        filename={props.filename}
        text={props.text}
        onClose={props.onClose}
      />
    );
  }
  return (
    <RecordFilePreviewContent
      key={`${props.clientId}:${props.filename}`}
      {...props}
    />
  );
}

function RecordFilePreviewContent({
  clientId,
  filename,
  readOnly = false,
  onClose,
}: {
  clientId: string;
  filename: string;
  readOnly?: boolean;
  onClose: () => void;
}) {
  const { data, loading, error } = useAsyncLoad(
    () => getRecordFileText(clientId, filename),
    [clientId, filename]
  );
  // Saved transcript edits replace the fetched text without a refetch.
  const [savedText, setSavedText] = useState<string | null>(null);
  const text = savedText ?? data;

  if (loading) {
    return (
      <Modal
        open
        onClose={onClose}
        title={filename}
        className="max-w-2xl p-6"
      >
        <div role="status" className="flex items-center gap-2 text-sm text-gray-500">
          <Spinner />
          Loading preview…
        </div>
      </Modal>
    );
  }

  if (error !== null) {
    return (
      <TextPreviewModal
        filename={filename}
        text={`Error loading preview: ${error}`}
        onClose={onClose}
      />
    );
  }

  return !readOnly && isAudioSidecar(filename) ? (
    <TranscriptEditor
      clientId={clientId}
      filename={filename}
      initialBody={text ?? ""}
      onClose={onClose}
      onSaved={setSavedText}
    />
  ) : (
    <TextPreviewModal filename={filename} text={text ?? ""} onClose={onClose} />
  );
}
