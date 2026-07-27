// Line-level diff with inline character highlighting.
//
// The diffing itself is delegated to jsdiff (Myers, O(ND)); this module only
// owns the shapes the version-compare view renders and the pairing pass that
// attaches character-level spans to changed line pairs.

import { diffArrays, diffChars } from "diff";

export type DiffSpan = {
  text: string;
  highlight: boolean;
};

export type DiffLine = {
  type: "add" | "remove" | "equal";
  line: string;
  /** When present, character-level spans within the line. */
  spans?: DiffSpan[];
};

/** Collapse consecutive spans that share a highlight flag. */
function mergeSpans(spans: DiffSpan[]): DiffSpan[] {
  const merged: DiffSpan[] = [];
  for (const span of spans) {
    if (span.text === "") continue;
    const last = merged[merged.length - 1];
    if (last && last.highlight === span.highlight) {
      last.text += span.text;
    } else {
      merged.push({ ...span });
    }
  }
  return merged;
}

/**
 * Compute a character-level diff between two strings.
 * Returns spans for both the old and new string.
 */
function charSpans(
  a: string,
  b: string,
): { removeSpans: DiffSpan[]; addSpans: DiffSpan[] } {
  const parts = diffChars(a, b);

  const removeSpans: DiffSpan[] = [];
  const addSpans: DiffSpan[] = [];
  for (const part of parts) {
    if (part.added) {
      addSpans.push({ text: part.value, highlight: true });
    } else if (part.removed) {
      removeSpans.push({ text: part.value, highlight: true });
    } else {
      removeSpans.push({ text: part.value, highlight: false });
      addSpans.push({ text: part.value, highlight: false });
    }
  }

  return {
    removeSpans: mergeSpans(removeSpans),
    addSpans: mergeSpans(addSpans),
  };
}

/**
 * Ceiling on the edit distance Myers will explore. Two versions of the same
 * file sit far below it; two texts with nothing in common would otherwise cost
 * O(n·m), and for those the wholesale replace-everything diff below is the
 * answer anyway.
 */
const MAX_EDIT_LENGTH = 2000;

/**
 * Compute a line-level diff between two strings.
 *
 * Returns an array of DiffLine entries representing the transformation
 * from `a` to `b`. Consecutive remove/add pairs get character-level
 * spans so the UI can highlight exactly which characters changed.
 */
export function diffLines(a: string, b: string): DiffLine[] {
  // Diff the pre-split line arrays rather than the raw text so a line is an
  // opaque token and trailing-newline handling stays ours.
  const aLines = a.split("\n");
  const bLines = b.split("\n");
  const changes = diffArrays(aLines, bLines, {
    maxEditLength: MAX_EDIT_LENGTH,
  }) ?? [
    { value: aLines, added: false, removed: true, count: aLines.length },
    { value: bLines, added: true, removed: false, count: bLines.length },
  ];

  const raw: DiffLine[] = [];
  for (const change of changes) {
    const type = change.added ? "add" : change.removed ? "remove" : "equal";
    for (const line of change.value) {
      raw.push({ type, line });
    }
  }

  // Post-process: pair consecutive remove/add runs and compute char-level spans
  const result: DiffLine[] = [];
  let k = 0;
  while (k < raw.length) {
    if (raw[k].type !== "remove") {
      result.push(raw[k]);
      k++;
      continue;
    }

    // Collect consecutive removes
    const removes: DiffLine[] = [];
    while (k < raw.length && raw[k].type === "remove") {
      removes.push(raw[k]);
      k++;
    }
    // Collect consecutive adds
    const adds: DiffLine[] = [];
    while (k < raw.length && raw[k].type === "add") {
      adds.push(raw[k]);
      k++;
    }

    // Pair them up for character-level diff
    const paired = Math.min(removes.length, adds.length);
    for (let p = 0; p < paired; p++) {
      const { removeSpans, addSpans } = charSpans(removes[p].line, adds[p].line);
      result.push({ ...removes[p], spans: removeSpans });
      result.push({ ...adds[p], spans: addSpans });
    }
    // Remaining unpaired lines
    for (let p = paired; p < removes.length; p++) {
      result.push(removes[p]);
    }
    for (let p = paired; p < adds.length; p++) {
      result.push(adds[p]);
    }
  }

  return result;
}
