import type {
  ReportBlock,
  ReportDraftEdit,
  ReportDraft,
  ReportSectionEdit,
  ReportSection,
} from "./tauri";

// ---------------------------------------------------------------------------
// Structural equality and block-level diffing — the one owner for "did this
// report content change", consumed by the edit counter and the proposal card.
// ---------------------------------------------------------------------------

/** Deep structural equality for two report blocks (either may be missing). */
export function blocksEqual(
  left: ReportBlock | undefined,
  right: ReportBlock | undefined
): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/** Structural equality for a section's heading and blocks. */
export function sectionsEqual(
  current: ReportSection,
  proposed: ReportSection
): boolean {
  return (
    current.heading === proposed.heading &&
    current.blocks.length === proposed.blocks.length &&
    current.blocks.every((block, index) =>
      blocksEqual(block, proposed.blocks[index])
    )
  );
}

export type BlockChange = {
  current: ReportBlock[];
  proposed: ReportBlock[];
  currentStart: number;
  proposedStart: number;
};

/**
 * Return only changed block runs. Exact unchanged paragraphs and lists are
 * aligned with an LCS and omitted entirely.
 */
export function diffBlocks(
  current: ReportBlock[],
  proposed: ReportBlock[]
): BlockChange[] {
  const rows = current.length + 1;
  const columns = proposed.length + 1;
  // Tables can contain thousands of cells. Serialize each block once rather
  // than repeating that work at every cell in the LCS matrix.
  const currentKeys = current.map((block) => JSON.stringify(block));
  const proposedKeys = proposed.map((block) => JSON.stringify(block));
  const lcs = Array.from({ length: rows }, () =>
    Array<number>(columns).fill(0)
  );

  for (let currentIndex = current.length - 1; currentIndex >= 0; currentIndex -= 1) {
    for (
      let proposedIndex = proposed.length - 1;
      proposedIndex >= 0;
      proposedIndex -= 1
    ) {
      lcs[currentIndex][proposedIndex] =
        currentKeys[currentIndex] === proposedKeys[proposedIndex]
        ? 1 + lcs[currentIndex + 1][proposedIndex + 1]
        : Math.max(
            lcs[currentIndex + 1][proposedIndex],
            lcs[currentIndex][proposedIndex + 1]
          );
    }
  }

  const changes: BlockChange[] = [];
  let currentIndex = 0;
  let proposedIndex = 0;
  let pending: BlockChange | null = null;
  const pendingChange = () => {
    pending ??= {
      current: [],
      proposed: [],
      currentStart: currentIndex,
      proposedStart: proposedIndex,
    };
    return pending;
  };
  const flush = () => {
    if (pending) changes.push(pending);
    pending = null;
  };

  while (currentIndex < current.length || proposedIndex < proposed.length) {
    if (
      currentIndex < current.length &&
      proposedIndex < proposed.length &&
      currentKeys[currentIndex] === proposedKeys[proposedIndex]
    ) {
      flush();
      currentIndex += 1;
      proposedIndex += 1;
    } else if (
      proposedIndex < proposed.length &&
      (currentIndex === current.length ||
        lcs[currentIndex][proposedIndex + 1] >=
          lcs[currentIndex + 1][proposedIndex])
    ) {
      pendingChange().proposed.push(proposed[proposedIndex]);
      proposedIndex += 1;
    } else {
      pendingChange().current.push(current[currentIndex]);
      currentIndex += 1;
    }
  }
  flush();
  return changes;
}

export function draftToEdit(draft: ReportDraft): ReportDraftEdit {
  return {
    title: draft.content.title,
    sections: draft.content.sections.map((section) => ({
      id: section.id,
      heading: section.heading,
      blocks: section.blocks.map(cloneBlock),
      skipped: section.skipped ?? false,
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
      skipped: section.skipped ?? false,
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
      if (!blocksEqual(before.blocks[blockIndex], after.blocks[blockIndex])) {
        count += 1;
      }
    }
  }
  return count;
}

export function newReportTable(): ReportBlock {
  return {
    kind: "table",
    rows: [
      ["Column 1", "Column 2"],
      ["", ""],
    ],
    has_header: true,
    column_widths: null,
  };
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
  let tableCells = 0;
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
      } else if (block.kind === "bullet_list") {
        if (block.items.length === 0 || block.items.length > 100) {
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
      } else {
        tableCells += block.rows.reduce((sum, row) => sum + row.length, 0);
        validateTable(errors, blockLabel, block);
      }
    });
  });
  if (tableCells > 20_000) {
    errors.push("Report tables can contain at most 20000 cells in total.");
  }
  return errors;
}

function validateTable(
  errors: string[],
  label: string,
  table: Extract<ReportBlock, { kind: "table" }>
) {
  if (table.rows.length === 0 || table.rows.length > 200) {
    errors.push(`${label} must contain 1 to 200 table rows.`);
    return;
  }
  const columns = table.rows[0].length;
  if (columns === 0 || columns > 20) {
    errors.push(`${label} must contain 1 to 20 table columns.`);
    return;
  }
  if (table.rows.some((row) => row.length !== columns)) {
    errors.push(`${label} must have the same number of cells in every row.`);
  }
  if (table.rows.flat().every((cell) => cell.trim() === "")) {
    errors.push(`${label} must contain at least one nonempty cell.`);
  }
  table.rows.forEach((row, rowIndex) =>
    row.forEach((cell, columnIndex) =>
      validateOptionalText(
        errors,
        `${label}, row ${rowIndex + 1}, column ${columnIndex + 1}`,
        cell,
        5_000
      )
    )
  );
  if (table.column_widths) {
    const widthTotal = table.column_widths.reduce((sum, width) => sum + width, 0);
    if (
      table.column_widths.length !== columns ||
      table.column_widths.some((width) => width <= 0) ||
      widthTotal !== 10_000
    ) {
      errors.push(
        `${label} column widths must contain one positive value per column and total 10000.`
      );
    }
  }
}

function validateOptionalText(
  errors: string[],
  label: string,
  value: string,
  maxCharacters: number
) {
  if (Array.from(value).length > maxCharacters) {
    errors.push(`${label} exceeds ${maxCharacters} characters.`);
  }
  if (containsInvalidWordCharacter(value)) {
    errors.push(`${label} contains a character Word cannot represent.`);
  }
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
  if (containsInvalidWordCharacter(value)) {
    errors.push(`${label} contains a character Word cannot represent.`);
  }
}

function containsInvalidWordCharacter(value: string): boolean {
  return Array.from(value).some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return !(
      codePoint === 0x09 ||
      codePoint === 0x0a ||
      codePoint === 0x0d ||
      (codePoint >= 0x20 && codePoint <= 0xd7ff) ||
      (codePoint >= 0xe000 && codePoint <= 0xfffd) ||
      (codePoint >= 0x10000 && codePoint <= 0x10ffff)
    );
  });
}

export function cloneBlock(block: ReportBlock): ReportBlock {
  if (block.kind === "paragraph") {
    return { kind: "paragraph", text: block.text };
  }
  if (block.kind === "bullet_list") {
    return { kind: "bullet_list", items: [...block.items] };
  }
  return {
    kind: "table",
    rows: block.rows.map((row) => [...row]),
    has_header: block.has_header,
    column_widths: block.column_widths ? [...block.column_widths] : null,
  };
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
