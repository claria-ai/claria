// Line-level diff with inline character highlighting.
// No external dependencies.

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

/**
 * Compute a character-level diff between two strings using LCS.
 * Returns spans for both the old and new string.
 */
function diffChars(
  a: string,
  b: string,
): { removeSpans: DiffSpan[]; addSpans: DiffSpan[] } {
  const n = a.length;
  const m = b.length;

  // Build LCS table
  const dp: number[][] = Array.from({ length: n + 1 }, () =>
    new Array(m + 1).fill(0),
  );

  for (let i = 1; i <= n; i++) {
    for (let j = 1; j <= m; j++) {
      if (a[i - 1] === b[j - 1]) {
        dp[i][j] = dp[i - 1][j - 1] + 1;
      } else {
        dp[i][j] = Math.max(dp[i - 1][j], dp[i][j - 1]);
      }
    }
  }

  // Backtrack to classify each character
  const aFlags: boolean[] = new Array(n).fill(true); // true = changed
  const bFlags: boolean[] = new Array(m).fill(true);

  let i = n;
  let j = m;
  while (i > 0 && j > 0) {
    if (a[i - 1] === b[j - 1]) {
      aFlags[i - 1] = false;
      bFlags[j - 1] = false;
      i--;
      j--;
    } else if (dp[i - 1][j] >= dp[i][j - 1]) {
      i--;
    } else {
      j--;
    }
  }

  // Collapse consecutive same-flag characters into spans
  function toSpans(str: string, flags: boolean[]): DiffSpan[] {
    const spans: DiffSpan[] = [];
    let k = 0;
    while (k < str.length) {
      const highlight = flags[k];
      let end = k;
      while (end < str.length && flags[end] === highlight) {
        end++;
      }
      spans.push({ text: str.slice(k, end), highlight });
      k = end;
    }
    return spans;
  }

  return {
    removeSpans: toSpans(a, aFlags),
    addSpans: toSpans(b, bFlags),
  };
}

/**
 * Compute a line-level diff between two strings.
 *
 * Returns an array of DiffLine entries representing the transformation
 * from `a` to `b`. Consecutive remove/add pairs get character-level
 * spans so the UI can highlight exactly which characters changed.
 */
export function diffLines(a: string, b: string): DiffLine[] {
  const aLines = a.split("\n");
  const bLines = b.split("\n");
  const n = aLines.length;
  const m = bLines.length;

  // Build LCS table
  const dp: number[][] = Array.from({ length: n + 1 }, () =>
    new Array(m + 1).fill(0),
  );

  for (let i = 1; i <= n; i++) {
    for (let j = 1; j <= m; j++) {
      if (aLines[i - 1] === bLines[j - 1]) {
        dp[i][j] = dp[i - 1][j - 1] + 1;
      } else {
        dp[i][j] = Math.max(dp[i - 1][j], dp[i][j - 1]);
      }
    }
  }

  // Backtrack to produce the diff
  const raw: DiffLine[] = [];
  let i = n;
  let j = m;

  while (i > 0 || j > 0) {
    if (i > 0 && j > 0 && aLines[i - 1] === bLines[j - 1]) {
      raw.push({ type: "equal", line: aLines[i - 1] });
      i--;
      j--;
    } else if (j > 0 && (i === 0 || dp[i][j - 1] >= dp[i - 1][j])) {
      raw.push({ type: "add", line: bLines[j - 1] });
      j--;
    } else {
      raw.push({ type: "remove", line: aLines[i - 1] });
      i--;
    }
  }

  raw.reverse();

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
      const { removeSpans, addSpans } = diffChars(
        removes[p].line,
        adds[p].line,
      );
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
