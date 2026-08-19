import type {
  CompletionCheckKind,
  CompletionReport,
  Finding,
  ReportContent,
} from "./tauri";

/** Which findings the list is showing. */
export type FindingsFilter = "all" | "style" | "consistency" | "resolved";

/**
 * How one finding is drawn. Derived on every render rather than read off
 * `finding.status` alone: the backend treats staleness as a question it
 * answers from the report, and the stored `invalidated` stamp is only a cache
 * of that answer. A finding whose section moved on since the review is
 * invalid here even if nothing has written the stamp yet.
 */
export type FindingState = "open" | "applied" | "dismissed" | "invalidated";

export type FindingGroup = {
  sectionId: string;
  /** The section's current heading, or the removed-section stand-in. */
  heading: string;
  /** Findings still open in this section, counted before any filtering. */
  openCount: number;
  findings: Finding[];
};

/** The section a finding points at is no longer in the report. */
const REMOVED_SECTION_HEADING = "Section no longer in the report";

/**
 * The backend names review properties in snake_case, and that is the name the
 * findings themselves carry. Rendering it as prose is presentation, so it
 * happens here rather than being sent over the bridge twice.
 */
export function reviewPropertyLabel(property: string): string {
  return property.replaceAll("_", " ");
}

/**
 * Whether the finding's anchor still describes the report — the frontend's
 * copy of the backend's `finding_is_stale`. A section that is gone is stale,
 * and so is one whose authorship stamp has moved past the revision the review
 * read. A section with no stamp has not been rewritten, so it is fresh.
 */
export function findingIsStale(
  finding: Finding,
  content: ReportContent
): boolean {
  const section = content.sections.find(
    (candidate) => candidate.id === finding.anchor.section_id
  );
  if (!section) return true;
  const authorship = section.authorship;
  return authorship ? authorship.revision > finding.anchor.revision : false;
}

/**
 * Resolution wins over staleness: an applied replacement keeps its receipt so
 * the undo stays reachable, and a dismissed finding stays dismissed. Only an
 * open finding can go stale.
 */
export function findingState(
  finding: Finding,
  content: ReportContent
): FindingState {
  if (finding.status === "applied") return "applied";
  if (finding.status === "dismissed") return "dismissed";
  if (finding.status === "invalidated") return "invalidated";
  return findingIsStale(finding, content) ? "invalidated" : "open";
}

/** The revision the anchored section is on now, for the staleness note. */
export function currentSectionRevision(
  finding: Finding,
  content: ReportContent,
  draftRevision: number
): number {
  const section = content.sections.find(
    (candidate) => candidate.id === finding.anchor.section_id
  );
  return section?.authorship?.revision ?? draftRevision;
}

export function matchesFilter(
  finding: Finding,
  state: FindingState,
  filter: FindingsFilter
): boolean {
  switch (filter) {
    case "all":
      return true;
    case "style":
      return finding.pass === "style";
    case "consistency":
      return finding.pass === "consistency";
    case "resolved":
      return state === "applied" || state === "dismissed";
  }
}

/**
 * Findings grouped by section in document order, filtered for display.
 *
 * `openCount` is counted before the filter runs, so the "Style" chip does not
 * make a section with three open consistency flags look clean. Findings whose
 * section has been deleted collect in one trailing group rather than
 * disappearing — a finding nobody can see is a finding nobody can dismiss.
 */
export function groupFindings(
  findings: readonly Finding[],
  content: ReportContent,
  filter: FindingsFilter
): FindingGroup[] {
  const openCounts = openFindingCounts(findings, content);
  const groups = new Map<string, FindingGroup>();
  for (const section of content.sections) {
    groups.set(section.id, {
      sectionId: section.id,
      heading: section.heading,
      openCount: openCounts.get(section.id) ?? 0,
      findings: [],
    });
  }
  const removed: FindingGroup = {
    sectionId: "",
    heading: REMOVED_SECTION_HEADING,
    openCount: 0,
    findings: [],
  };
  for (const finding of findings) {
    if (!matchesFilter(finding, findingState(finding, content), filter)) {
      continue;
    }
    (groups.get(finding.anchor.section_id) ?? removed).findings.push(finding);
  }
  const ordered = [...groups.values()].filter(
    (group) => group.findings.length > 0
  );
  if (removed.findings.length > 0) ordered.push(removed);
  return ordered;
}

/**
 * Open findings per section, for the canvas flag chips. Derived from the
 * findings themselves rather than from run state: a review can be long over
 * by the time the reader looks at the document.
 */
export function openFindingCounts(
  findings: readonly Finding[],
  content: ReportContent
): ReadonlyMap<string, number> {
  const counts = new Map<string, number>();
  for (const finding of findings) {
    if (findingState(finding, content) !== "open") continue;
    const sectionId = finding.anchor.section_id;
    counts.set(sectionId, (counts.get(sectionId) ?? 0) + 1);
  }
  return counts;
}

/**
 * The passage the finding points at, sliced out of the anchored section.
 *
 * `null` when the span is missing or no longer addresses a paragraph — the
 * card then shows the description alone rather than an invented quote.
 */
export function anchoredQuote(
  finding: Finding,
  content: ReportContent
): string | null {
  const span = finding.span;
  if (!span) return null;
  const section = content.sections.find(
    (candidate) => candidate.id === finding.anchor.section_id
  );
  const block = section?.blocks[span.block_index];
  if (!block || block.kind !== "paragraph") return null;
  const quote = block.text.slice(span.start_char, span.end_char).trim();
  return quote === "" ? null : quote;
}

/** The heading of the section a consistency finding conflicts with. */
export function conflictingHeading(
  finding: Finding,
  content: ReportContent
): string | null {
  const sectionId = finding.conflicting?.section_id;
  if (!sectionId) return null;
  return (
    content.sections.find((candidate) => candidate.id === sectionId)?.heading ??
    null
  );
}

export type CompletionSummaryRow = {
  kind: CompletionCheckKind;
  /** One line, already counted and pluralised. */
  label: string;
  /** Headings of the sections at fault, in document order. */
  sections: string[];
};

const COMPLETION_LABELS: Record<
  CompletionCheckKind,
  (count: number) => string
> = {
  section_not_terminal: (count) =>
    count === 1
      ? "1 section never finished drafting"
      : `${count} sections never finished drafting`,
  required_section_empty: (count) =>
    count === 1
      ? "1 required section is still empty"
      : `${count} required sections are still empty`,
  unresolved_citation: (count) =>
    count === 1
      ? "1 citation could not be verified"
      : `${count} citations could not be verified`,
  missing_citation: (count) =>
    count === 1
      ? "1 section cites no records"
      : `${count} sections cite no records`,
  placeholder_text: (count) =>
    count === 1
      ? "1 section still has placeholder text"
      : `${count} sections still have placeholder text`,
  unresolved_finding: (count) =>
    count === 1 ? "1 open finding" : `${count} open findings`,
};

/** The order the checklist reads in, worst-structured problem first. */
const COMPLETION_ORDER: CompletionCheckKind[] = [
  "section_not_terminal",
  "required_section_empty",
  "placeholder_text",
  "missing_citation",
  "unresolved_citation",
  "unresolved_finding",
];

/**
 * One line per failing check kind, counted and named. Checks with no section
 * (a placeholder left in the title, say) contribute to the count without
 * naming anything.
 */
export function summarizeCompletion(
  report: CompletionReport,
  content: ReportContent
): CompletionSummaryRow[] {
  const headings = new Map(
    content.sections.map((section) => [section.id, section.heading])
  );
  const byKind = new Map<CompletionCheckKind, string[]>();
  const counts = new Map<CompletionCheckKind, number>();
  for (const check of report.checks) {
    counts.set(check.kind, (counts.get(check.kind) ?? 0) + 1);
    const heading = check.section_id
      ? headings.get(check.section_id)
      : undefined;
    if (heading === undefined) continue;
    const named = byKind.get(check.kind) ?? [];
    if (!named.includes(heading)) named.push(heading);
    byKind.set(check.kind, named);
  }
  return COMPLETION_ORDER.filter((kind) => counts.has(kind)).map((kind) => ({
    kind,
    label: COMPLETION_LABELS[kind](counts.get(kind) ?? 0),
    sections: byKind.get(kind) ?? [],
  }));
}
