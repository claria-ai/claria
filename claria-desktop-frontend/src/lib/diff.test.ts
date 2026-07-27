import { describe, expect, it } from "vitest";
import { diffLines } from "./diff";

// This module is an adapter over jsdiff, so what is worth pinning is the
// shape it hands the version-compare view: the line sequence, the pairing of
// removes with adds, and the character spans hung off those pairs.

const types = (lines: ReturnType<typeof diffLines>) => lines.map((l) => l.type);
const text = (lines: ReturnType<typeof diffLines>) => lines.map((l) => l.line);

describe("diffLines", () => {
  it("marks identical text as all equal", () => {
    const result = diffLines("one\ntwo", "one\ntwo");
    expect(types(result)).toEqual(["equal", "equal"]);
    expect(result.every((l) => l.spans === undefined)).toBe(true);
  });

  it("keeps every line of both sides", () => {
    const result = diffLines("a\nb\nc", "a\nx\nc");
    expect(text(result)).toContain("b");
    expect(text(result)).toContain("x");
  });

  it("orders a changed line as remove then add", () => {
    const result = diffLines("a\nb\nc", "a\nx\nc");
    expect(types(result)).toEqual(["equal", "remove", "add", "equal"]);
  });

  it("reports a pure insertion with no removes", () => {
    const result = diffLines("a\nc", "a\nb\nc");
    expect(types(result)).toEqual(["equal", "add", "equal"]);
  });

  it("reports a pure deletion with no adds", () => {
    const result = diffLines("a\nb\nc", "a\nc");
    expect(types(result)).toEqual(["equal", "remove", "equal"]);
  });

  it("treats an empty document as one empty line, not zero lines", () => {
    // "".split("\n") is [""], so an empty side still has a line to diff.
    expect(diffLines("", "").length).toBe(1);
  });
});

describe("character spans", () => {
  it("highlights only the characters that changed in a paired line", () => {
    const result = diffLines("the cat sat", "the dog sat");
    const removed = result.find((l) => l.type === "remove");
    const added = result.find((l) => l.type === "add");

    expect(removed?.spans?.map((s) => s.text).join("")).toBe("the cat sat");
    expect(added?.spans?.map((s) => s.text).join("")).toBe("the dog sat");

    expect(removed?.spans?.filter((s) => s.highlight).map((s) => s.text)).toEqual(["cat"]);
    expect(added?.spans?.filter((s) => s.highlight).map((s) => s.text)).toEqual(["dog"]);
  });

  it("merges neighbouring spans that share a highlight flag", () => {
    const result = diffLines("abcdef", "abXYef");
    const removed = result.find((l) => l.type === "remove");
    // No two adjacent spans may carry the same flag, or the view renders
    // gratuitously fragmented markup.
    const flags = removed?.spans?.map((s) => s.highlight) ?? [];
    for (let i = 1; i < flags.length; i++) {
      expect(flags[i]).not.toBe(flags[i - 1]);
    }
  });

  it("drops empty spans", () => {
    const result = diffLines("abc", "abcdef");
    for (const line of result) {
      for (const span of line.spans ?? []) {
        expect(span.text).not.toBe("");
      }
    }
  });

  it("attaches spans only to lines that are part of a remove/add pair", () => {
    // Two removes against one add: the first pairs, the second is unpaired.
    const result = diffLines("a\nb\nc", "x");
    const removes = result.filter((l) => l.type === "remove");
    expect(removes.length).toBe(3);
    expect(removes[0].spans).toBeDefined();
    expect(removes[1].spans).toBeUndefined();
    expect(removes[2].spans).toBeUndefined();
  });

  it("leaves unrelated lines without spans", () => {
    const result = diffLines("a\nb", "a\nx");
    expect(result.find((l) => l.type === "equal")?.spans).toBeUndefined();
  });
});

describe("edit-length ceiling", () => {
  it("falls back to replacing everything when two texts share nothing", () => {
    // Past `maxEditLength` jsdiff returns undefined, and the module has to
    // substitute a whole-file replacement rather than crash the compare view.
    const a = Array.from({ length: 4000 }, (_, i) => `left ${i}`).join("\n");
    const b = Array.from({ length: 4000 }, (_, i) => `right ${i}`).join("\n");
    const result = diffLines(a, b);

    expect(result.some((l) => l.type === "remove")).toBe(true);
    expect(result.some((l) => l.type === "add")).toBe(true);
    expect(result.some((l) => l.type === "equal")).toBe(false);
    expect(result.filter((l) => l.type === "remove").length).toBe(4000);
    expect(result.filter((l) => l.type === "add").length).toBe(4000);
  });
});
