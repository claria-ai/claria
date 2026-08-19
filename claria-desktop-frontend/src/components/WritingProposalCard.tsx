import { InlineMarkdown } from "./Markdown";
import {
  Blocks,
  Change,
  Comparison,
  EmptyPreview,
  PlainText,
  SectionPreview,
  TablePreview,
  type TableBlock,
} from "./WritingDiff";
import {
  diffBlocks,
  sectionsEqual,
  type BlockChange,
} from "../lib/writingWorkspace";
import type {
  ReportContent,
  ReportProposalView,
  ReportSection,
} from "../lib/tauri";

export default function WritingProposalCard({
  proposal,
  accepted,
  busy,
  onAccept,
  onReject,
}: {
  proposal: ReportProposalView;
  accepted: ReportContent;
  busy: boolean;
  onAccept: () => void;
  onReject: () => void;
}) {
  return (
    <section
      data-testid="report-proposal"
      aria-label="Pending report proposal"
      className="border border-violet-200 bg-violet-50 rounded-lg p-4 space-y-4"
    >
      <div>
        <p className="text-xs font-semibold uppercase tracking-wide text-violet-700">
          Proposed by Claude
        </p>
        <h4 className="text-sm font-semibold text-gray-900 mt-1">
          <InlineMarkdown text={proposal.summary} />
        </h4>
        <p className="text-xs text-gray-500 mt-1">
          Based on accepted revision {proposal.base_revision}. Only fields that
          would actually change are shown below.
        </p>
      </div>

      <ProposalChanges
        accepted={accepted}
        proposed={proposal.proposed_content}
      />

      <div className="flex justify-end gap-2 pt-1">
        <button
          type="button"
          onClick={onReject}
          disabled={busy}
          className="px-3 py-2 text-xs font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 disabled:opacity-50"
        >
          {busy ? "Working…" : "Reject"}
        </button>
        <button
          type="button"
          onClick={onAccept}
          disabled={busy}
          className="px-3 py-2 text-xs font-medium text-white bg-violet-600 rounded-md hover:bg-violet-700 disabled:opacity-50"
        >
          {busy ? "Working…" : "Accept & save"}
        </button>
      </div>
    </section>
  );
}

function ProposalChanges({
  accepted,
  proposed,
}: {
  accepted: ReportContent;
  proposed: ReportContent;
}) {
  const currentSections = new Map(
    accepted.sections.map((section) => [section.id, section])
  );
  const proposedSections = new Map(
    proposed.sections.map((section) => [section.id, section])
  );
  const titleChanged = accepted.title !== proposed.title;
  const changedOrAdded = proposed.sections.filter((section) => {
    const current = currentSections.get(section.id);
    return !current || !sectionsEqual(current, section);
  });
  const removed = accepted.sections.filter(
    (section) => !proposedSections.has(section.id)
  );

  if (!titleChanged && changedOrAdded.length === 0 && removed.length === 0) {
    return (
      <p className="text-xs text-gray-500 bg-white border border-violet-100 rounded-md p-3">
        This proposal has no net report changes.
      </p>
    );
  }

  return (
    <div className="space-y-3">
      {titleChanged && (
        <Change label="Change title">
          <Comparison
            current={<PlainText text={accepted.title} />}
            proposed={<PlainText text={proposed.title} />}
          />
        </Change>
      )}

      {changedOrAdded.map((section) => {
        const current = currentSections.get(section.id);
        return current ? (
          <ChangedSection
            key={section.id}
            current={current}
            proposed={section}
          />
        ) : (
          <Change key={section.id} label="Add section">
            <SectionPreview section={section} />
          </Change>
        );
      })}

      {removed.map((section) => (
        <Change key={section.id} label="Remove section">
          <SectionPreview section={section} tone="removed" />
        </Change>
      ))}
    </div>
  );
}

function ChangedSection({
  current,
  proposed,
}: {
  current: ReportSection;
  proposed: ReportSection;
}) {
  const headingChanged = current.heading !== proposed.heading;
  const blockChanges = diffBlocks(current.blocks, proposed.blocks);

  return (
    <Change label={`Change section · ${proposed.heading}`}>
      <div className="space-y-3">
        {headingChanged && (
          <div>
            <p className="text-[11px] font-semibold text-gray-600 mb-1">
              Heading
            </p>
            <Comparison
              current={<PlainText text={current.heading} />}
              proposed={<PlainText text={proposed.heading} />}
            />
          </div>
        )}
        {blockChanges.map((change, index) => {
          const tablePair = pairedTableChange(change);
          return (
            <div key={`${change.currentStart}:${change.proposedStart}:${index}`}>
              <p className="text-[11px] font-semibold text-gray-600 mb-1">
                {blockChangeLabel(change)}
              </p>
              <Comparison
                current={
                  tablePair ? (
                    <TablePreview
                      table={tablePair.current}
                      comparison={tablePair.proposed}
                      tone="current"
                    />
                  ) : change.current.length > 0 ? (
                    <Blocks blocks={change.current} />
                  ) : (
                    <EmptyPreview />
                  )
                }
                proposed={
                  tablePair ? (
                    <TablePreview
                      table={tablePair.proposed}
                      comparison={tablePair.current}
                      tone="proposed"
                    />
                  ) : change.proposed.length > 0 ? (
                    <Blocks blocks={change.proposed} />
                  ) : (
                    <EmptyPreview />
                  )
                }
              />
            </div>
          );
        })}
      </div>
    </Change>
  );
}

function pairedTableChange(
  change: BlockChange
): { current: TableBlock; proposed: TableBlock } | null {
  const current = change.current[0];
  const proposed = change.proposed[0];
  return change.current.length === 1 &&
    change.proposed.length === 1 &&
    current.kind === "table" &&
    proposed.kind === "table"
    ? { current, proposed }
    : null;
}

function blockChangeLabel(change: BlockChange): string {
  if (change.current.length === 1 && change.proposed.length === 1) {
    const block = change.current[0];
    if (block.kind === "table" && change.proposed[0].kind === "table") {
      const cells = changedTableCells(block, change.proposed[0]);
      return cells === 0
        ? `Change table ${change.currentStart + 1} settings`
        : `Change table ${change.currentStart + 1} · ${cells} cell${cells === 1 ? "" : "s"}`;
    }
    const kind = block.kind === "paragraph" ? "paragraph" : "bullet list";
    return `Change ${kind} ${change.currentStart + 1}`;
  }
  if (change.current.length === 0) {
    return `Add ${formatBlockCount(change.proposed.length)} at position ${change.proposedStart + 1}`;
  }
  if (change.proposed.length === 0) {
    return `Remove ${formatBlockCount(change.current.length)} at position ${change.currentStart + 1}`;
  }
  return "Change section content";
}

function formatBlockCount(count: number): string {
  return `${count} block${count === 1 ? "" : "s"}`;
}

function changedTableCells(current: TableBlock, proposed: TableBlock): number {
  let changed = 0;
  const rows = Math.max(current.rows.length, proposed.rows.length);
  for (let rowIndex = 0; rowIndex < rows; rowIndex += 1) {
    const columns = Math.max(
      current.rows[rowIndex]?.length ?? 0,
      proposed.rows[rowIndex]?.length ?? 0
    );
    for (let columnIndex = 0; columnIndex < columns; columnIndex += 1) {
      if (
        current.rows[rowIndex]?.[columnIndex] !==
        proposed.rows[rowIndex]?.[columnIndex]
      ) {
        changed += 1;
      }
    }
  }
  return changed;
}
