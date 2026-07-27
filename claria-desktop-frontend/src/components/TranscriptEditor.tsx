import { useMemo, useState } from "react";
import { saveTranscriptEdits } from "../lib/tauri";
import {
  formatTime,
  parseBody,
  renderBody,
  type Segment,
} from "../lib/transcript";
import Modal from "./Modal";

/**
 * Per-segment transcript editor.
 *
 * Parses the `.text` sidecar body into segments (label, time range, text,
 * optional translation), exposes them as editable rows, and writes back the
 * rendered body on save. S3 object versioning preserves every prior body
 * (including the original Transcribe output as v1). Restore is handled by
 * the standard Version History modal (time icon on the file row), which
 * for audio files targets the `.text` sidecar — every revision is listed
 * and individually restorable.
 *
 * Parsing and rendering live in `lib/transcript.ts`, which mirrors the Rust
 * implementation in `claria-transcribe`. Hand-edited speaker labels propagate
 * via label-string identity (rename "Speaker 1" → "Clinician" and every
 * `[Speaker 1 ...]` header in the body flips).
 *
 * Restoring the original Transcribe output is handled by the standard
 * Version History modal (time icon on the file row), which is wired to
 * the `.text` sidecar for audio files — every transcript revision is
 * listed there, including v1.
 *
 * Legacy header-less bodies (PDF/DOCX sidecars, older audio transcripts)
 * parse as a single un-diarized segment so this editor remains usable on any
 * `.text` file.
 */
export default function TranscriptEditor({
  clientId,
  filename,
  initialBody,
  onClose,
  onSaved,
}: {
  clientId: string;
  filename: string;
  initialBody: string;
  onClose: () => void;
  onSaved: (newBody: string) => void;
}) {
  const initial = useMemo(() => parseBody(initialBody), [initialBody]);
  const [segments, setSegments] = useState<Segment[]>(initial.segments);
  const [labelByKey, setLabelByKey] = useState<Map<string, string>>(
    initial.labelByKey
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function handleSegmentTextChange(idx: number, text: string) {
    setSegments((prev) =>
      prev.map((s, i) => (i === idx ? { ...s, text } : s))
    );
  }

  function handleTranslationChange(idx: number, translation: string) {
    setSegments((prev) =>
      prev.map((s, i) =>
        i === idx
          ? { ...s, translation: translation.length > 0 ? translation : null }
          : s
      )
    );
  }

  function handleRenameSpeaker(speakerKey: string, newLabel: string) {
    setLabelByKey((prev) => {
      const next = new Map(prev);
      next.set(speakerKey, newLabel);
      return next;
    });
  }

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      const body = renderBody(segments, labelByKey);
      await saveTranscriptEdits(clientId, filename, body);
      onSaved(body);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  // Distinct speaker keys in stable order (first-seen).
  const speakerKeys: string[] = [];
  for (const seg of segments) {
    if (seg.speakerKey != null && !speakerKeys.includes(seg.speakerKey)) {
      speakerKeys.push(seg.speakerKey);
    }
  }

  const isStructured = segments.some(
    (s) => s.speakerKey != null || s.languageCode != null
  );

  return (
    <Modal
      open
      onClose={onClose}
      title={filename}
      variant="framed"
      className="max-w-3xl max-h-[90vh] flex flex-col"
      dismissible={!saving}
    >
      <div className="flex-1 overflow-y-auto px-5 py-4 space-y-4">
        {/* Speaker rename pane — only when the body has diarization. */}
        {speakerKeys.length > 0 && (
          <div className="bg-gray-50 border border-gray-200 rounded-lg p-3">
            <p className="text-xs font-medium text-gray-700 mb-2">Speakers</p>
            <p className="text-xs text-gray-500 mb-2">
              Rename here once and every header in the transcript updates on
              save.
            </p>
            <div className="space-y-1.5">
              {speakerKeys.map((key) => (
                <div key={key} className="flex items-center gap-2">
                  <span className="text-xs text-gray-400 w-16">{key}</span>
                  <input
                    type="text"
                    value={labelByKey.get(key) ?? key}
                    onChange={(e) => handleRenameSpeaker(key, e.target.value)}
                    className="flex-1 px-2 py-1 text-sm border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-blue-500"
                  />
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Segments */}
        {isStructured ? (
          <div className="space-y-3">
            {segments.map((seg, idx) => (
              <SegmentRow
                key={seg.id}
                seg={seg}
                label={
                  seg.speakerKey != null
                    ? labelByKey.get(seg.speakerKey) ?? seg.speakerKey
                    : null
                }
                onTextChange={(t) => handleSegmentTextChange(idx, t)}
                onTranslationChange={(t) => handleTranslationChange(idx, t)}
              />
            ))}
          </div>
        ) : (
          // Header-less body: single textarea, no speaker pane.
          <textarea
            value={segments[0]?.text ?? ""}
            onChange={(e) => handleSegmentTextChange(0, e.target.value)}
            className="w-full min-h-[300px] px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono resize-y focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        )}

        {error && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-3">
            <p className="text-red-800 text-sm">{error}</p>
          </div>
        )}
      </div>

      <div className="px-5 py-3 border-t border-gray-200 flex items-center justify-end gap-2">
        <button
          onClick={onClose}
          disabled={saving}
          className="px-3 py-1.5 text-sm text-gray-700 hover:bg-gray-100 rounded-lg disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          onClick={handleSave}
          disabled={saving}
          className="px-3 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
        >
          {saving ? "Saving..." : "Save"}
        </button>
      </div>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Per-segment row
// ---------------------------------------------------------------------------

function SegmentRow({
  seg,
  label,
  onTextChange,
  onTranslationChange,
}: {
  seg: Segment;
  label: string | null;
  onTextChange: (t: string) => void;
  onTranslationChange: (t: string) => void;
}) {
  return (
    <div className="border border-gray-200 rounded-lg p-3">
      <div className="flex items-center gap-2 mb-1.5 text-xs text-gray-500">
        {label && <span className="font-medium text-gray-700">{label}</span>}
        <span>
          {formatTime(seg.startSeconds)}–{formatTime(seg.endSeconds)}
        </span>
        {seg.languageCode && (
          <span className="px-1.5 py-0.5 bg-gray-100 rounded text-gray-600">
            {seg.languageCode}
          </span>
        )}
      </div>
      <textarea
        value={seg.text}
        onChange={(e) => onTextChange(e.target.value)}
        rows={Math.max(2, Math.ceil(seg.text.length / 60))}
        className="w-full px-2 py-1.5 text-sm border border-gray-200 rounded resize-y focus:outline-none focus:ring-1 focus:ring-blue-500"
      />
      {seg.translation != null && (
        <textarea
          value={seg.translation}
          onChange={(e) => onTranslationChange(e.target.value)}
          rows={Math.max(2, Math.ceil(seg.translation.length / 60))}
          className="mt-1.5 w-full px-2 py-1.5 text-xs text-gray-600 italic border-l-2 border-gray-300 bg-gray-50 rounded resize-y focus:outline-none focus:ring-1 focus:ring-blue-500"
          placeholder="Translation"
        />
      )}
    </div>
  );
}
