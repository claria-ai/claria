import type { ReactNode } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type {
  ReportBlockView,
  ReportContentView,
  ReportProposalView,
  ReportSectionView,
} from "../lib/tauri";

export default function WritingProposalCard({
  proposal,
  accepted,
  busy,
  onAccept,
  onReject,
}: {
  proposal: ReportProposalView;
  accepted: ReportContentView;
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
  accepted: ReportContentView;
  proposed: ReportContentView;
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
  current: ReportSectionView;
  proposed: ReportSectionView;
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

type TableBlock = Extract<ReportBlockView, { kind: "table" }>;

type BlockChange = {
  current: ReportBlockView[];
  proposed: ReportBlockView[];
  currentStart: number;
  proposedStart: number;
};

/**
 * Return only changed block runs. Exact unchanged paragraphs and lists are
 * aligned with an LCS and omitted from the proposal card entirely.
 */
function diffBlocks(
  current: ReportBlockView[],
  proposed: ReportBlockView[]
): BlockChange[] {
  const rows = current.length + 1;
  const columns = proposed.length + 1;
  // Tables can contain thousands of cells. Serialize each block once rather
  // than repeating that work at every cell in the LCS matrix.
  const currentKeys = current.map((block) => JSON.stringify(block));
  const proposedKeys = proposed.map((block) => JSON.stringify(block));
  const lcs = Array.from({ length: rows }, () =>
    Array<number>(columns).fill(0)
  );

  for (let currentIndex = current.length - 1; currentIndex >= 0; currentIndex -= 1) {
    for (
      let proposedIndex = proposed.length - 1;
      proposedIndex >= 0;
      proposedIndex -= 1
    ) {
      lcs[currentIndex][proposedIndex] =
        currentKeys[currentIndex] === proposedKeys[proposedIndex]
        ? 1 + lcs[currentIndex + 1][proposedIndex + 1]
        : Math.max(
            lcs[currentIndex + 1][proposedIndex],
            lcs[currentIndex][proposedIndex + 1]
          );
    }
  }

  const changes: BlockChange[] = [];
  let currentIndex = 0;
  let proposedIndex = 0;
  let pending: BlockChange | null = null;
  const pendingChange = () => {
    pending ??= {
      current: [],
      proposed: [],
      currentStart: currentIndex,
      proposedStart: proposedIndex,
    };
    return pending;
  };
  const flush = () => {
    if (pending) changes.push(pending);
    pending = null;
  };

  while (currentIndex < current.length || proposedIndex < proposed.length) {
    if (
      currentIndex < current.length &&
      proposedIndex < proposed.length &&
      currentKeys[currentIndex] === proposedKeys[proposedIndex]
    ) {
      flush();
      currentIndex += 1;
      proposedIndex += 1;
    } else if (
      proposedIndex < proposed.length &&
      (currentIndex === current.length ||
        lcs[currentIndex][proposedIndex + 1] >=
          lcs[currentIndex + 1][proposedIndex])
    ) {
      pendingChange().proposed.push(proposed[proposedIndex]);
      proposedIndex += 1;
    } else {
      pendingChange().current.push(current[currentIndex]);
      currentIndex += 1;
    }
  }
  flush();
  return changes;
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

function sectionsEqual(
  current: ReportSectionView,
  proposed: ReportSectionView
): boolean {
  return (
    current.heading === proposed.heading &&
    current.blocks.length === proposed.blocks.length &&
    current.blocks.every((block, index) =>
      blocksEqual(block, proposed.blocks[index])
    )
  );
}

function blocksEqual(
  current: ReportBlockView,
  proposed: ReportBlockView
): boolean {
  return JSON.stringify(current) === JSON.stringify(proposed);
}

function Change({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="bg-white border border-violet-100 rounded-md p-3">
      <p className="text-xs font-semibold text-gray-800 mb-2">{label}</p>
      {children}
    </div>
  );
}

function Comparison({
  current,
  proposed,
}: {
  current: ReactNode;
  proposed: ReactNode;
}) {
  return (
    <div className="grid grid-cols-2 gap-3">
      <div>
        <p className="text-[11px] font-medium text-gray-500 mb-1">Current</p>
        {current}
      </div>
      <div>
        <p className="text-[11px] font-medium text-violet-700 mb-1">Proposed</p>
        {proposed}
      </div>
    </div>
  );
}

function SectionPreview({
  section,
  tone = "proposed",
}: {
  section: ReportSectionView;
  tone?: "proposed" | "removed";
}) {
  return (
    <div
      className={`border rounded p-2 bg-white ${
        tone === "removed" ? "border-red-200" : "border-violet-200"
      }`}
    >
      <p className="text-xs font-semibold text-gray-900">
        <InlineMarkdown text={section.heading} />
      </p>
      <Blocks blocks={section.blocks} />
    </div>
  );
}

function Blocks({ blocks }: { blocks: ReportBlockView[] }) {
  return (
    <div className="border border-gray-200 rounded p-2 bg-white mt-1.5 space-y-1 text-xs leading-5 text-gray-700">
      {blocks.map((block, index) => {
        if (block.kind === "paragraph") {
          return (
            <div key={index} className="prose prose-xs max-w-none prose-p:my-1">
              <Markdown remarkPlugins={[remarkGfm]}>{block.text}</Markdown>
            </div>
          );
        }
        if (block.kind === "bullet_list") {
          return (
            <ul key={index} className="list-disc pl-4">
              {block.items.map((item, itemIndex) => (
                <li key={itemIndex}>
                  <InlineMarkdown text={item} />
                </li>
              ))}
            </ul>
          );
        }
        return <TablePreview key={index} table={block} />;
      })}
    </div>
  );
}

function TablePreview({
  table,
  comparison,
  tone = "neutral",
}: {
  table: TableBlock;
  comparison?: TableBlock;
  tone?: "current" | "proposed" | "neutral";
}) {
  const layoutChanged =
    comparison !== undefined &&
    (table.has_header !== comparison.has_header ||
      JSON.stringify(table.column_widths) !==
        JSON.stringify(comparison.column_widths));
  return (
    <div className="mt-1.5 overflow-x-auto rounded border border-gray-200 bg-white">
      {layoutChanged && (
        <p className="border-b border-gray-200 bg-amber-50 px-2 py-1 text-[10px] text-amber-800">
          Header or column layout changed
        </p>
      )}
      <table className="w-full border-collapse text-[11px] leading-4">
        {table.column_widths && (
          <colgroup>
            {table.column_widths.map((width, index) => (
              <col key={index} style={{ width: `${width / 100}%` }} />
            ))}
          </colgroup>
        )}
        <tbody>
          {table.rows.map((row, rowIndex) => (
            <tr
              key={rowIndex}
              className={table.has_header && rowIndex === 0 ? "font-semibold" : ""}
            >
              {row.map((cell, columnIndex) => {
                const changed =
                  comparison !== undefined &&
                  comparison.rows[rowIndex]?.[columnIndex] !== cell;
                return (
                  <td
                    key={columnIndex}
                    className={`border-b border-r last:border-r-0 border-gray-200 px-1.5 py-1 whitespace-pre-wrap align-top ${
                      changed
                        ? tone === "current"
                          ? "bg-red-50 text-red-900"
                          : "bg-violet-100 text-violet-950"
                        : table.has_header && rowIndex === 0
                          ? "bg-slate-100"
                          : ""
                    }`}
                  >
                    {cell || <span className="italic text-gray-400">blank</span>}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
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

function EmptyPreview() {
  return (
    <div className="border border-dashed border-gray-200 rounded p-2 text-xs italic text-gray-400">
      Nothing
    </div>
  );
}

function PlainText({ text }: { text: string }) {
  return (
    <div className="text-xs text-gray-700 border border-gray-200 rounded p-2 bg-white prose prose-xs max-w-none prose-p:my-1">
      <Markdown remarkPlugins={[remarkGfm]}>{text}</Markdown>
    </div>
  );
}

function InlineMarkdown({ text }: { text: string }) {
  return (
    <Markdown
      remarkPlugins={[remarkGfm]}
      components={{ p: ({ children }) => <>{children}</> }}
    >
      {text}
    </Markdown>
  );
}
