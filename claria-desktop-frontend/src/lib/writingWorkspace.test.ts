import { describe, expect, it } from "vitest";
import {
  cloneReportEdit,
  draftToEdit,
  moveItem,
  newReportSection,
  newReportTable,
  reportEditsEqual,
  validateReportEdit,
} from "./writingWorkspace";
import type { ReportDraftView } from "./tauri";

const draft: ReportDraftView = {
  revision: 2,
  content: {
    title: "Report",
    sections: [
      {
        id: "section-1",
        heading: "Summary",
        blocks: [
          { kind: "paragraph", text: "Text" },
          { kind: "bullet_list", items: ["One", "Two"] },
          {
            kind: "table",
            rows: [["Measure", "Score"], ["Attention", "87"]],
            has_header: true,
            column_widths: [7000, 3000],
          },
        ],
      },
    ],
  },
  created_at: "2026-08-01T00:00:00Z",
  updated_at: "2026-08-02T00:00:00Z",
  last_applied_proposal_id: null,
};

describe("report edit helpers", () => {
  it("clones the accepted draft without sharing nested block arrays", () => {
    const edit = draftToEdit(draft);
    const clone = cloneReportEdit(edit);
    expect(reportEditsEqual(edit, clone)).toBe(true);

    const bullets = clone.sections[0].blocks[1];
    if (bullets.kind !== "bullet_list") throw new Error("expected bullets");
    bullets.items[0] = "Changed";

    const original = edit.sections[0].blocks[1];
    if (original.kind !== "bullet_list") throw new Error("expected bullets");
    expect(original.items[0]).toBe("One");
    expect(reportEditsEqual(edit, clone)).toBe(false);

    const table = clone.sections[0].blocks[2];
    if (table.kind !== "table") throw new Error("expected table");
    table.rows[1][1] = "91";
    const originalTable = edit.sections[0].blocks[2];
    if (originalTable.kind !== "table") throw new Error("expected table");
    expect(originalTable.rows[1][1]).toBe("87");
  });

  it("creates sections without client-generated IDs", () => {
    const section = newReportSection();
    expect(section.id).toBeNull();
    expect(section.blocks[0]).toEqual({
      kind: "paragraph",
      text: "New paragraph",
    });
    expect(
      validateReportEdit({ title: "Report", sections: [section] })
    ).toEqual([]);
  });

  it("creates a rectangular editable table", () => {
    const table = newReportTable();
    expect(table).toEqual({
      kind: "table",
      rows: [["Column 1", "Column 2"], ["", ""]],
      has_header: true,
      column_widths: null,
    });
    expect(
      validateReportEdit({
        title: "Report",
        sections: [{ id: null, heading: "Scores", blocks: [table] }],
      })
    ).toEqual([]);
  });

  it("reports malformed tables and Word-incompatible cells before save", () => {
    const errors = validateReportEdit({
      title: "Report",
      sections: [
        {
          id: null,
          heading: "Scores",
          blocks: [
            {
              kind: "table",
              rows: [["A", "bad\u000b"], ["B"]],
              has_header: true,
              column_widths: [5000, 4999],
            },
          ],
        },
      ],
    });
    expect(errors).toEqual(
      expect.arrayContaining([
        "Section 1, block 1 must have the same number of cells in every row.",
        "Section 1, block 1, row 1, column 2 contains a character Word cannot represent.",
        "Section 1, block 1 column widths must contain one positive value per column and total 10000.",
      ])
    );
  });

  it("caps aggregate table cells before save", () => {
    const rows = Array.from({ length: 200 }, (_, rowIndex) =>
      Array.from({ length: 20 }, (_, columnIndex) =>
        rowIndex === 0 && columnIndex === 0 ? "value" : ""
      )
    );
    const errors = validateReportEdit({
      title: "Report",
      sections: [
        {
          id: null,
          heading: "Tables",
          blocks: Array.from({ length: 6 }, () => ({
            kind: "table" as const,
            rows,
            has_header: false,
            column_widths: null,
          })),
        },
      ],
    });
    expect(errors).toContain(
      "Report tables can contain at most 20000 cells in total."
    );
  });

  it("reports empty and Word-incompatible fields before save", () => {
    expect(
      validateReportEdit({
        title: " ",
        sections: [
          {
            id: null,
            heading: "",
            blocks: [
              { kind: "paragraph", text: "" },
              { kind: "bullet_list", items: ["valid", "bad\u000bitem"] },
            ],
          },
        ],
      })
    ).toEqual(
      expect.arrayContaining([
        "Report title must not be empty.",
        "Section 1 heading must not be empty.",
        "Section 1, block 1 must not be empty.",
        "Section 1, block 2, bullet 2 contains a character Word cannot represent.",
      ])
    );
  });

  it("moves ordered items without mutating the input", () => {
    const input = ["a", "b", "c"];
    expect(moveItem(input, 0, 2)).toEqual(["b", "c", "a"]);
    expect(input).toEqual(["a", "b", "c"]);
    expect(moveItem(input, -1, 1)).toBe(input);
  });
});
