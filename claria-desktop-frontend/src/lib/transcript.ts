// Parser and renderer for the `.text` transcript sidecar body.
//
// This is one half of a two-language mirror: `crates/claria-transcribe/src/
// lib.rs::{parse_transcript_body, format_transcript_body}` implements the same
// grammar in Rust, and the two must agree — the Rust side writes the body when
// a transcription lands, this side reads and rewrites it when the user edits.
//
// Body grammar:
//
//   [<label> <mm:ss>–<mm:ss>[ <language_code>]]
//   <segment text, one or more lines>
//   > <translation, one line per source line>
//
// Bodies with no headers at all (PDF/DOCX sidecars, transcripts predating
// diarization) parse as a single un-diarized segment.

export interface Segment {
  id: string;
  speakerKey: string | null;
  languageCode: string | null;
  startSeconds: number;
  endSeconds: number;
  text: string;
  translation: string | null;
}

export interface ParsedBody {
  segments: Segment[];
  /**
   * Map from the speaker key (interned label-string identity) to the current
   * display label. Initially key === label; renames in the editor update the
   * map without touching segments.
   */
  labelByKey: Map<string, string>;
}

export const HEADER_RE =
  /^\[(.+?)\s+(\d{1,2}:\d{2})[–—-](\d{1,2}:\d{2})(?:\s+([\w-]+))?\]$/;

export function parseBody(body: string): ParsedBody {
  const segments: Segment[] = [];
  const labelByKey = new Map<string, string>();
  const lines = body.split("\n");

  let counter = 1;
  let i = 0;
  while (i < lines.length) {
    const line = lines[i].trim();
    if (line.length === 0) {
      i += 1;
      continue;
    }
    const match = line.match(HEADER_RE);
    if (match) {
      const label = match[1].trim();
      const startSeconds = mmSsToSeconds(match[2]);
      const endSeconds = mmSsToSeconds(match[3]);
      const lang = match[4] ?? null;
      i += 1;
      const textLines: string[] = [];
      const translationLines: string[] = [];
      while (i < lines.length && lines[i].trim().match(HEADER_RE) == null) {
        const raw = lines[i];
        const trimmed = raw.trimStart();
        if (trimmed.startsWith("> ")) {
          translationLines.push(trimmed.slice(2));
        } else if (trimmed === ">") {
          translationLines.push("");
        } else {
          textLines.push(raw);
        }
        i += 1;
      }
      const speakerKey = label.length > 0 ? label : null;
      if (speakerKey != null && !labelByKey.has(speakerKey)) {
        labelByKey.set(speakerKey, speakerKey);
      }
      segments.push({
        id: `seg_${String(counter).padStart(4, "0")}`,
        speakerKey,
        languageCode: lang,
        startSeconds,
        endSeconds,
        text: textLines.join("\n").trim(),
        translation:
          translationLines.length === 0
            ? null
            : translationLines.join("\n").trim(),
      });
      counter += 1;
    } else {
      // Header-less body: consume everything as one segment.
      const text = lines.slice(i).join("\n").trim();
      if (text.length > 0) {
        segments.push({
          id: `seg_${String(counter).padStart(4, "0")}`,
          speakerKey: null,
          languageCode: null,
          startSeconds: 0,
          endSeconds: 0,
          text,
          translation: null,
        });
      }
      break;
    }
  }

  return { segments, labelByKey };
}

export function renderBody(
  segments: Segment[],
  labelByKey: Map<string, string>
): string {
  const allUnstructured = segments.every(
    (s) => s.speakerKey == null && s.languageCode == null
  );
  if (allUnstructured) {
    return segments.map((s) => s.text).join("\n\n");
  }

  const parts: string[] = [];
  for (const seg of segments) {
    const label =
      seg.speakerKey != null
        ? labelByKey.get(seg.speakerKey) ?? seg.speakerKey
        : "Speaker";
    const start = formatTime(seg.startSeconds);
    const end = formatTime(seg.endSeconds);
    const lang = seg.languageCode != null ? ` ${seg.languageCode}` : "";
    parts.push(`[${label} ${start}–${end}${lang}]`);
    parts.push(seg.text.trim());
    if (seg.translation != null && seg.translation.trim().length > 0) {
      for (const line of seg.translation.trim().split("\n")) {
        parts.push(`> ${line}`);
      }
    }
    parts.push("");
  }
  return parts.join("\n").trimEnd();
}

export function mmSsToSeconds(s: string): number {
  const [mm, ss] = s.split(":");
  const mins = Number(mm);
  const secs = Number(ss);
  return mins * 60 + secs;
}

export function formatTime(totalSeconds: number): string {
  const m = Math.floor(totalSeconds / 60);
  const s = totalSeconds % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}
