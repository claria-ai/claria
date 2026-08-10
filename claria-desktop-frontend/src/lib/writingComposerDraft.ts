import type { ReportBlock } from "./tauri";

export type WritingBlockReference = {
  kind: "paragraph" | "table";
  sectionId: string;
  blockIndex: number;
  sectionHeading: string;
  preview: string;
};

export type WritingComposerDraft = {
  instruction: string;
  references: WritingBlockReference[];
};

// Composer drafts can contain PHI, so keep them in process memory rather than
// localStorage. They survive tab/page navigation but disappear when Claria
// closes.
const drafts = new Map<string, WritingComposerDraft>();

export function readWritingComposerDraft(
  clientId: string
): WritingComposerDraft | null {
  const draft = drafts.get(clientId);
  return draft ? cloneDraft(draft) : null;
}

export function writeWritingComposerDraft(
  clientId: string,
  draft: WritingComposerDraft
): void {
  if (draft.instruction === "" && draft.references.length === 0) {
    drafts.delete(clientId);
    return;
  }
  drafts.set(clientId, cloneDraft(draft));
}

export function clearWritingComposerDrafts(): void {
  drafts.clear();
}

type ReferenceableReportBlock =
  | Extract<ReportBlock, { kind: "paragraph" }>
  | Extract<ReportBlock, { kind: "table" }>;

export function reportBlockReferencePreview(
  block: ReferenceableReportBlock
): string {
  const text =
    block.kind === "paragraph"
      ? block.text
      : block.rows
          .map((row) =>
            row
              .map((cell) => cell.replace(/\s+/g, " ").trim())
              .filter(Boolean)
              .join(" | ")
          )
          .filter(Boolean)
          .join(" · ");
  const compact = text.replace(/\s+/g, " ").trim();
  if (compact === "") return "Empty table";
  return compact.length > 90 ? `${compact.slice(0, 87)}…` : compact;
}

function cloneDraft(draft: WritingComposerDraft): WritingComposerDraft {
  return {
    instruction: draft.instruction,
    references: draft.references.map((reference) => ({ ...reference })),
  };
}
