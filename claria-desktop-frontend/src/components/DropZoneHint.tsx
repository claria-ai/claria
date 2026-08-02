import { transcribeSummary, transcribeTooltip } from "../lib/transcribe";
import type { TranscriptionPreferences } from "../lib/tauri";

/**
 * Footer of the file panel, doubling as the drop target's affordance.
 *
 * It grows when the record is empty so a new record has an obvious place to
 * drop the first file, and it names the transcription settings that a dropped
 * audio file will be processed with — those are set elsewhere, and this is
 * the moment the user needs to know them.
 */
export default function DropZoneHint({
  dragging,
  prefs,
  spacious,
}: {
  dragging: boolean;
  prefs: TranscriptionPreferences | null;
  /** Give the zone extra height, for a record with nothing in it yet. */
  spacious: boolean;
}) {
  return (
    <div
      className={`px-4 py-6 text-center ${spacious ? "py-12" : ""}`}
      title={transcribeTooltip(prefs)}
    >
      <p
        className={`text-sm ${
          dragging ? "text-blue-600 font-medium" : "text-gray-400"
        }`}
      >
        {dragging
          ? "Drop files to upload"
          : "Drag files here — PDF, DOCX, audio, or text"}
      </p>
      {!dragging && prefs && (
        <p className="text-xs text-gray-400 mt-1">{transcribeSummary(prefs)}</p>
      )}
    </div>
  );
}
