export type WritingParagraphReference = {
  sectionId: string;
  blockIndex: number;
  sectionHeading: string;
  preview: string;
};

export type WritingComposerDraft = {
  instruction: string;
  references: WritingParagraphReference[];
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

function cloneDraft(draft: WritingComposerDraft): WritingComposerDraft {
  return {
    instruction: draft.instruction,
    references: draft.references.map((reference) => ({ ...reference })),
  };
}
