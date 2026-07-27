import { describe, expect, it } from "vitest";
import { defaultMemoStamp } from "./useMemoRecorder";

/**
 * The recorder's one pure part. It seeds the filename the user is shown after
 * a recording, so it has to sort and pad predictably — `memo-2026-3-1-9-5`
 * would sort wrongly next to its neighbours in the file list.
 */
describe("defaultMemoStamp", () => {
  it("pads every field to a fixed width", () => {
    // Local time; month is 0-based in the Date constructor.
    expect(defaultMemoStamp(new Date(2026, 2, 1, 9, 5))).toBe("20260301-0905");
  });

  it("keeps two-digit fields intact", () => {
    expect(defaultMemoStamp(new Date(2026, 10, 25, 14, 30))).toBe(
      "20261125-1430",
    );
  });

  it("renders midnight as 0000, not blank", () => {
    expect(defaultMemoStamp(new Date(2026, 0, 1, 0, 0))).toBe("20260101-0000");
  });

  it("sorts lexicographically in chronological order", () => {
    const stamps = [
      defaultMemoStamp(new Date(2026, 11, 31, 23, 59)),
      defaultMemoStamp(new Date(2026, 0, 2, 0, 0)),
      defaultMemoStamp(new Date(2026, 0, 1, 9, 5)),
    ];
    expect([...stamps].sort()).toEqual([
      "20260101-0905",
      "20260102-0000",
      "20261231-2359",
    ]);
  });
});
