import type { ReactNode } from "react";
import type {
  ReportBlockView,
  ReportContentView,
  ReportProposalView,
  ReportSectionView,
} from "../lib/tauri";

export default function ReportProposalCard({
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
          {proposal.summary}
        </h4>
        <p className="text-xs text-gray-500 mt-1">
          Based on accepted revision {proposal.base_revision}. All changes are
          applied together only after approval.
        </p>
      </div>

      <Change label="Complete accepted vs final report">
        <Comparison
          current={<ContentPreview content={accepted} />}
          proposed={
            <div data-testid="proposal-final-candidate">
              <ContentPreview content={proposal.proposed_content} />
            </div>
          }
        />
      </Change>

      <div className="space-y-3">
        {proposal.operations.map((operation, index) => {
          if (operation.kind === "set_title") {
            return (
              <Change key={index} label="Change title">
                <Comparison
                  current={<PlainText text={accepted.title} />}
                  proposed={<PlainText text={operation.title} />}
                />
              </Change>
            );
          }
          if (operation.kind === "add_section") {
            return (
              <Change
                key={index}
                label={`Add section at position ${operation.position + 1}`}
              >
                <div>
                  <p className="text-[11px] font-medium text-violet-700 mb-1">
                    Proposed
                  </p>
                  <SectionPreview section={operation.section} />
                </div>
              </Change>
            );
          }
          const current = accepted.sections.find(
            (section) => section.id === operation.section_id
          );
          if (operation.kind === "replace_section") {
            const proposed: ReportSectionView = {
              id: operation.section_id,
              heading: operation.heading,
              blocks: operation.blocks,
            };
            return (
              <Change key={index} label="Replace section">
                <Comparison
                  current={
                    current ? (
                      <SectionPreview section={current} />
                    ) : (
                      <PlainText text="Section no longer exists" />
                    )
                  }
                  proposed={<SectionPreview section={proposed} />}
                />
              </Change>
            );
          }
          return (
            <Change key={index} label="Remove section">
              <div>
                <p className="text-[11px] font-medium text-red-700 mb-1">
                  Proposed deletion
                </p>
                {current ? (
                  <SectionPreview section={current} />
                ) : (
                  <PlainText text="Section no longer exists" />
                )}
              </div>
            </Change>
          );
        })}
      </div>

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

function ContentPreview({ content }: { content: ReportContentView }) {
  return (
    <article className="border border-gray-200 rounded p-2 bg-white space-y-2">
      <p className="text-xs font-bold text-gray-900">{content.title}</p>
      {content.sections.length === 0 ? (
        <p className="text-xs italic text-gray-400">No sections</p>
      ) : (
        content.sections.map((section) => (
          <SectionPreview key={section.id} section={section} />
        ))
      )}
    </article>
  );
}

function SectionPreview({ section }: { section: ReportSectionView }) {
  return (
    <div className="border border-gray-200 rounded p-2 bg-white">
      <p className="text-xs font-semibold text-gray-900">{section.heading}</p>
      <Blocks blocks={section.blocks} />
    </div>
  );
}

function Blocks({ blocks }: { blocks: ReportBlockView[] }) {
  return (
    <div className="mt-1.5 space-y-1 text-xs leading-5 text-gray-700">
      {blocks.length === 0 && <p className="italic text-gray-400">No blocks</p>}
      {blocks.map((block, index) =>
        block.kind === "paragraph" ? (
          <p key={index} className="whitespace-pre-wrap">
            {block.text}
          </p>
        ) : (
          <ul key={index} className="list-disc pl-4">
            {block.items.map((item, itemIndex) => (
              <li key={itemIndex}>{item}</li>
            ))}
          </ul>
        )
      )}
    </div>
  );
}

function PlainText({ text }: { text: string }) {
  return (
    <p className="text-xs text-gray-700 whitespace-pre-wrap border border-gray-200 rounded p-2 bg-white">
      {text}
    </p>
  );
}
