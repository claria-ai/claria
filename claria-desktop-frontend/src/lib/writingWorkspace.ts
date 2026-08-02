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

export function newReportTable(): ReportBlockView {
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
  table: Extract<ReportBlockView, { kind: "table" }>
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

export function cloneBlock(block: ReportBlockView): ReportBlockView {
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
