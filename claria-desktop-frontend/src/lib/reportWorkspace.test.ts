import { describe, expect, it } from "vitest";
import {
  cloneReportEdit,
  draftToEdit,
  moveItem,
  newReportSection,
  reportEditsEqual,
  validateReportEdit,
} from "./reportWorkspace";
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
