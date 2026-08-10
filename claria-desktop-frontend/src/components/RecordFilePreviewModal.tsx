import { useEffect, useState } from "react";
import { getRecordFileText } from "../lib/tauri";
import { isAudioSidecar } from "../lib/recordFiles";
import Modal from "./Modal";
import Spinner from "./Spinner";
import TextPreviewModal from "./TextPreviewModal";
import TranscriptEditor from "./TranscriptEditor";

/**
 * Load and display one record file exactly as the Record eye action does.
 * Writing context pills reuse this component so every file preview follows the
 * same sidecar lookup, loading, error, and transcript behavior.
 */
export default function RecordFilePreviewModal(props: {
  clientId: string;
  filename: string;
  onClose: () => void;
}) {
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
  onClose,
}: {
  clientId: string;
  filename: string;
  onClose: () => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    void getRecordFileText(clientId, filename).then(
      (value) => {
        if (current) setText(value);
      },
      (reason) => {
        if (current) setError(String(reason));
      }
    );
    return () => {
      current = false;
    };
  }, [clientId, filename]);

  if (text === null && error === null) {
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

  return isAudioSidecar(filename) ? (
    <TranscriptEditor
      clientId={clientId}
      filename={filename}
      initialBody={text ?? ""}
      onClose={onClose}
      onSaved={setText}
    />
  ) : (
    <TextPreviewModal filename={filename} text={text ?? ""} onClose={onClose} />
  );
}
