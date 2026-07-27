import { describe, expect, it } from "vitest";
import { normalizeForSearch, searchMatches } from "./search";

describe("normalizeForSearch", () => {
  it("strips diacritics and lowercases", () => {
    expect(normalizeForSearch("Lucía")).toBe("lucia");
    expect(normalizeForSearch("MÜLLER")).toBe("muller");
    expect(normalizeForSearch("Ångström")).toBe("angstrom");
  });

  it("folds precomposed and decomposed spellings to the same string", () => {
    // "é" as one code point vs. "e" + combining acute. A name pasted from a
    // PDF and the same name typed by hand differ this way.
    expect(normalizeForSearch("café")).toBe(normalizeForSearch("café"));
  });

  it("leaves letters that are not accented forms alone", () => {
    // ø and ß are distinct letters, not decomposable — folding them would
    // need a transliteration table this deliberately does not have.
    expect(normalizeForSearch("Søren")).toBe("søren");
    expect(normalizeForSearch("Straße")).toBe("straße");
  });

  it("keeps non-Latin scripts intact", () => {
    expect(normalizeForSearch("Иванов")).toBe("иванов");
    expect(normalizeForSearch("中文")).toBe("中文");
  });
});

describe("searchMatches", () => {
  it("matches an unaccented query against an accented name", () => {
    expect(searchMatches("Lucía Fernández", "luci")).toBe(true);
    expect(searchMatches("Lucía Fernández", "fernandez")).toBe(true);
  });

  it("matches an accented query against an unaccented name", () => {
    expect(searchMatches("Lucia Fernandez", "Lucía")).toBe(true);
  });

  it("ignores case on both sides", () => {
    expect(searchMatches("Lucía", "LUCÍA")).toBe(true);
  });

  it("matches in the middle of a string, not just the start", () => {
    expect(searchMatches("Maria de la Cruz", "de la")).toBe(true);
  });

  it("is false when the query is genuinely absent", () => {
    expect(searchMatches("Lucía Fernández", "gonzalez")).toBe(false);
  });

  it("matches everything on an empty or whitespace query", () => {
    expect(searchMatches("Lucía", "")).toBe(true);
    expect(searchMatches("Lucía", "   ")).toBe(true);
  });

  it("trims the query but not the haystack", () => {
    expect(searchMatches("Lucía", "  luc  ")).toBe(true);
  });
});
