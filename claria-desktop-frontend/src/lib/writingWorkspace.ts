import type {
  ReportBlockView,
  ReportDraftEdit,
  ReportDraftView,
  ReportSectionEdit,
} from "./tauri";

export function draftToEdit(draft: ReportDraftView): ReportDraftEdit {
  return {
    title: draft.content.title,
    sections: draft.content.sections.map((section) => ({
      id: section.id,
      heading: section.heading,
      blocks: section.blocks.map(cloneBlock),
    })),
  };
}

export function cloneReportEdit(edit: ReportDraftEdit): ReportDraftEdit {
  return {
    title: edit.title,
    sections: edit.sections.map((section) => ({
      id: section.id,
      heading: section.heading,
      blocks: section.blocks.map(cloneBlock),
    })),
  };
}

export function reportEditsEqual(
  left: ReportDraftEdit,
  right: ReportDraftEdit
): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/** Approximate user-facing count of fields/blocks changed from the saved draft. */
export function countReportEdits(
  baseline: ReportDraftEdit,
  edit: ReportDraftEdit
): number {
  let count = baseline.title === edit.title ? 0 : 1;
  const sectionCount = Math.max(baseline.sections.length, edit.sections.length);
  for (let sectionIndex = 0; sectionIndex < sectionCount; sectionIndex += 1) {
    const before = baseline.sections[sectionIndex];
    const after = edit.sections[sectionIndex];
    if (!before || !after) {
      count += 1;
      continue;
    }
    if (before.id !== after.id || before.heading !== after.heading) count += 1;
    const blockCount = Math.max(before.blocks.length, after.blocks.length);
    for (let blockIndex = 0; blockIndex < blockCount; blockIndex += 1) {
      if (
        JSON.stringify(before.blocks[blockIndex]) !==
        JSON.stringify(after.blocks[blockIndex])
      ) {
        count += 1;
      }
    }
  }
  return count;
}

export function newReportSection(): ReportSectionEdit {
  return {
    id: null,
    heading: "New section",
    blocks: [{ kind: "paragraph", text: "New paragraph" }],
  };
}

export function validateReportEdit(edit: ReportDraftEdit): string[] {
  const errors: string[] = [];
  validateRequiredText(errors, "Report title", edit.title, 200);
  if (edit.sections.length > 100) {
    errors.push("A report can contain at most 100 sections.");
  }
  edit.sections.forEach((section, sectionIndex) => {
    const sectionLabel = `Section ${sectionIndex + 1}`;
    validateRequiredText(errors, `${sectionLabel} heading`, section.heading, 200);
    if (section.blocks.length > 200) {
      errors.push(`${sectionLabel} can contain at most 200 blocks.`);
    }
    section.blocks.forEach((block, blockIndex) => {
      const blockLabel = `${sectionLabel}, block ${blockIndex + 1}`;
      if (block.kind === "paragraph") {
        validateRequiredText(errors, blockLabel, block.text, 20_000);
      } else if (block.items.length === 0 || block.items.length > 100) {
        errors.push(`${blockLabel} must contain 1 to 100 bullet items.`);
      } else {
        block.items.forEach((item, itemIndex) =>
          validateRequiredText(
            errors,
            `${blockLabel}, bullet ${itemIndex + 1}`,
            item,
            2_000
          )
        );
      }
    });
  });
  return errors;
}

function validateRequiredText(
  errors: string[],
  label: string,
  value: string,
  maxCharacters: number
) {
  if (value.trim() === "") {
    errors.push(`${label} must not be empty.`);
    return;
  }
  if (Array.from(value).length > maxCharacters) {
    errors.push(`${label} exceeds ${maxCharacters} characters.`);
  }
  if (
    Array.from(value).some((character) => {
      const codePoint = character.codePointAt(0) ?? 0;
      return !(
        codePoint === 0x09 ||
        codePoint === 0x0a ||
        codePoint === 0x0d ||
        (codePoint >= 0x20 && codePoint <= 0xd7ff) ||
        (codePoint >= 0xe000 && codePoint <= 0xfffd) ||
        (codePoint >= 0x10000 && codePoint <= 0x10ffff)
      );
    })
  ) {
    errors.push(`${label} contains a character Word cannot represent.`);
  }
}

export function cloneBlock(block: ReportBlockView): ReportBlockView {
  return block.kind === "paragraph"
    ? { kind: "paragraph", text: block.text }
    : { kind: "bullet_list", items: [...block.items] };
}

export function moveItem<T>(items: T[], from: number, to: number): T[] {
  if (
    from < 0 ||
    to < 0 ||
    from >= items.length ||
    to >= items.length ||
    from === to
  ) {
    return items;
  }
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}
