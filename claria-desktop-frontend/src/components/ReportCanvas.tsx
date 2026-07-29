import type {
  ReportBlockView,
  ReportDraftEdit,
  ReportSectionEdit,
  ReportWorkspaceView,
} from "../lib/tauri";
import { moveItem, newReportSection } from "../lib/reportWorkspace";

export default function ReportCanvas({
  workspace,
  edit,
  editing,
  dirty,
  busy,
  onBeginEdit,
  onCancelEdit,
  onChange,
  onSave,
  onExport,
  saveStatus,
  exportStatus,
  validationErrors,
}: {
  workspace: ReportWorkspaceView;
  edit: ReportDraftEdit;
  editing: boolean;
  dirty: boolean;
  busy: boolean;
  onBeginEdit: () => void;
  onCancelEdit: () => void;
  onChange: (edit: ReportDraftEdit) => void;
  onSave: () => void;
  onExport: () => void;
  saveStatus: string | null;
  exportStatus: string | null;
  validationErrors: string[];
}) {
  const pending = workspace.pending_proposal !== null;

  function updateSection(index: number, section: ReportSectionEdit) {
    const sections = [...edit.sections];
    sections[index] = section;
    onChange({ ...edit, sections });
  }

  function updateBlock(
    sectionIndex: number,
    blockIndex: number,
    block: ReportBlockView
  ) {
    const section = edit.sections[sectionIndex];
    const blocks = [...section.blocks];
    blocks[blockIndex] = block;
    updateSection(sectionIndex, { ...section, blocks });
  }

  function removeBlock(sectionIndex: number, blockIndex: number) {
    const section = edit.sections[sectionIndex];
    updateSection(sectionIndex, {
      ...section,
      blocks: section.blocks.filter((_, index) => index !== blockIndex),
    });
  }

  return (
    <section
      aria-label="Accepted report draft"
      className="h-full min-h-[32rem] flex flex-col bg-gray-50 min-[800px]:border-l border-gray-200"
    >
      <div className="px-5 py-3 bg-white border-b border-gray-200 flex items-center gap-3">
        <div className="flex-1">
          <h3 className="text-sm font-semibold text-gray-900">Accepted report</h3>
          <p className="text-xs text-gray-500">
            Revision {workspace.draft.revision}
            {dirty ? " · Unsaved changes" : " · Saved"}
          </p>
        </div>
        {!editing ? (
          <button
            type="button"
            onClick={onBeginEdit}
            disabled={busy || pending}
            className="px-3 py-1.5 text-xs font-medium border border-gray-300 rounded-md bg-white hover:bg-gray-50 disabled:opacity-50"
          >
            Edit
          </button>
        ) : (
          <>
            <button
              type="button"
              onClick={onCancelEdit}
              disabled={busy}
              className="px-3 py-1.5 text-xs text-gray-600 hover:text-gray-900 disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={onSave}
              disabled={busy || pending || !dirty || validationErrors.length > 0}
              className="px-3 py-1.5 text-xs font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50"
            >
              {busy ? "Working…" : "Save Draft"}
            </button>
          </>
        )}
        <button
          type="button"
          onClick={onExport}
          disabled={busy || dirty}
          title={dirty ? "Save or discard edits before exporting" : undefined}
          className="px-3 py-1.5 text-xs font-medium text-white bg-emerald-600 rounded-md hover:bg-emerald-700 disabled:opacity-50"
        >
          Export .docx
        </button>
      </div>

      <div className="px-5 pt-3 space-y-2">
        <p className="text-[11px] leading-4 text-amber-800 bg-amber-50 border border-amber-200 rounded px-3 py-2">
          Local exports may contain PHI and are not encrypted or managed by
          Claria. Store and share the .docx according to your organization&apos;s
          privacy and security requirements.
        </p>
        {(saveStatus || exportStatus) && (
          <p role="status" aria-live="polite" className="text-xs text-gray-600">
            {saveStatus ?? exportStatus}
          </p>
        )}
      </div>

      <div className="flex-1 overflow-y-auto p-6">
        <div className="max-w-3xl mx-auto bg-white min-h-full border border-gray-200 shadow-sm rounded-sm px-10 py-12">
          {editing ? (
            <>
              {validationErrors.length > 0 && (
                <div role="alert" className="mb-5 border border-red-200 bg-red-50 rounded-md p-3">
                  <p className="text-xs font-semibold text-red-800">
                    Fix these report fields before saving:
                  </p>
                  <ul className="mt-1 list-disc pl-5 text-xs text-red-700">
                    {validationErrors.map((error) => (
                      <li key={error}>{error}</li>
                    ))}
                  </ul>
                </div>
              )}
              <EditableReport
              edit={edit}
              disabled={busy || pending}
              onChange={onChange}
              updateSection={updateSection}
              updateBlock={updateBlock}
              removeBlock={removeBlock}
            />
            </>
          ) : (
            <AcceptedReport workspace={workspace} />
          )}
        </div>
      </div>
    </section>
  );
}

function AcceptedReport({ workspace }: { workspace: ReportWorkspaceView }) {
  const content = workspace.draft.content;
  return (
    <article data-testid="accepted-report-canvas">
      <h1 className="text-3xl font-semibold text-center text-gray-900 mb-10">
        {content.title}
      </h1>
      {content.sections.length === 0 ? (
        <div className="border border-dashed border-gray-300 rounded-lg p-8 text-center">
          <p className="text-sm text-gray-500">The accepted report has no sections yet.</p>
          <p className="text-xs text-gray-400 mt-1">
            Ask the assistant for a proposal or choose Edit.
          </p>
        </div>
      ) : (
        <div className="space-y-8">
          {content.sections.map((section) => (
            <section key={section.id}>
              <h2 className="text-xl font-semibold text-gray-900 mb-3">
                {section.heading}
              </h2>
              <div className="space-y-3 text-sm leading-6 text-gray-700">
                {section.blocks.map((block, index) =>
                  block.kind === "paragraph" ? (
                    <div key={index} className="space-y-3">
                      {block.text.split("\n").map((line, lineIndex) => (
                        <p key={lineIndex}>{line || "\u00a0"}</p>
                      ))}
                    </div>
                  ) : (
                    <ul key={index} className="list-disc pl-6 space-y-1">
                      {block.items.map((item, itemIndex) => (
                        <li key={itemIndex}>{item}</li>
                      ))}
                    </ul>
                  )
                )}
              </div>
            </section>
          ))}
        </div>
      )}
    </article>
  );
}

function EditableReport({
  edit,
  disabled,
  onChange,
  updateSection,
  updateBlock,
  removeBlock,
}: {
  edit: ReportDraftEdit;
  disabled: boolean;
  onChange: (edit: ReportDraftEdit) => void;
  updateSection: (index: number, section: ReportSectionEdit) => void;
  updateBlock: (
    sectionIndex: number,
    blockIndex: number,
    block: ReportBlockView
  ) => void;
  removeBlock: (sectionIndex: number, blockIndex: number) => void;
}) {
  return (
    <div className="space-y-8">
      <label className="block">
        <span className="text-xs font-medium text-gray-500">Report title</span>
        <input
          aria-label="Report title"
          value={edit.title}
          onChange={(event) => onChange({ ...edit, title: event.target.value })}
          disabled={disabled}
          className="mt-1 w-full text-2xl font-semibold text-center border border-gray-300 rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
        />
      </label>

      {edit.sections.map((section, sectionIndex) => (
        <section
          key={section.id ?? `new-${sectionIndex}`}
          className="border border-gray-200 rounded-lg p-4 space-y-4"
        >
          <div className="flex gap-2 items-start">
            <label className="flex-1">
              <span className="sr-only">Section heading</span>
              <input
                aria-label={`Section ${sectionIndex + 1} heading`}
                value={section.heading}
                onChange={(event) =>
                  updateSection(sectionIndex, {
                    ...section,
                    heading: event.target.value,
                  })
                }
                disabled={disabled}
                className="w-full text-lg font-semibold border border-gray-300 rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
              />
            </label>
            <ReorderButtons
              label="section"
              index={sectionIndex}
              length={edit.sections.length}
              disabled={disabled}
              onMove={(to) =>
                onChange({
                  ...edit,
                  sections: moveItem(edit.sections, sectionIndex, to),
                })
              }
            />
            <button
              type="button"
              onClick={() =>
                onChange({
                  ...edit,
                  sections: edit.sections.filter(
                    (_, index) => index !== sectionIndex
                  ),
                })
              }
              disabled={disabled}
              className="px-2 py-1.5 text-xs text-red-600 hover:bg-red-50 rounded disabled:opacity-50"
            >
              Remove
            </button>
          </div>

          <div className="space-y-3">
            {section.blocks.map((block, blockIndex) => (
              <div key={blockIndex} className="flex gap-2 items-start">
                <div className="flex-1">
                  <div className="flex items-center gap-2 mb-1">
                    <select
                      aria-label={`Section ${sectionIndex + 1} block ${blockIndex + 1} type`}
                      value={block.kind}
                      disabled={disabled}
                      onChange={(event) =>
                        updateBlock(
                          sectionIndex,
                          blockIndex,
                          event.target.value === "paragraph"
                            ? { kind: "paragraph", text: "New paragraph" }
                            : { kind: "bullet_list", items: ["New bullet"] }
                        )
                      }
                      className="text-xs border border-gray-300 rounded px-2 py-1 bg-white"
                    >
                      <option value="paragraph">Paragraph</option>
                      <option value="bullet_list">Bullet list</option>
                    </select>
                  </div>
                  {block.kind === "paragraph" ? (
                    <textarea
                      aria-label={`Section ${sectionIndex + 1} paragraph ${blockIndex + 1}`}
                      value={block.text}
                      onChange={(event) =>
                        updateBlock(sectionIndex, blockIndex, {
                          kind: "paragraph",
                          text: event.target.value,
                        })
                      }
                      disabled={disabled}
                      rows={4}
                      className="w-full border border-gray-300 rounded-md px-3 py-2 text-sm resize-y focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
                    />
                  ) : (
                    <textarea
                      aria-label={`Section ${sectionIndex + 1} bullet items ${blockIndex + 1}`}
                      value={block.items.join("\n")}
                      onChange={(event) =>
                        updateBlock(sectionIndex, blockIndex, {
                          kind: "bullet_list",
                          items: event.target.value.split("\n"),
                        })
                      }
                      disabled={disabled}
                      rows={4}
                      placeholder="One bullet per line"
                      className="w-full border border-gray-300 rounded-md px-3 py-2 text-sm resize-y focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
                    />
                  )}
                </div>
                <ReorderButtons
                  label="block"
                  index={blockIndex}
                  length={section.blocks.length}
                  disabled={disabled}
                  onMove={(to) =>
                    updateSection(sectionIndex, {
                      ...section,
                      blocks: moveItem(section.blocks, blockIndex, to),
                    })
                  }
                />
                <button
                  type="button"
                  onClick={() => removeBlock(sectionIndex, blockIndex)}
                  disabled={disabled}
                  className="mt-6 px-2 py-1.5 text-xs text-red-600 hover:bg-red-50 rounded disabled:opacity-50"
                >
                  Remove
                </button>
              </div>
            ))}
          </div>

          <div className="flex gap-2">
            <button
              type="button"
              disabled={disabled}
              onClick={() =>
                updateSection(sectionIndex, {
                  ...section,
                  blocks: [
                    ...section.blocks,
                    { kind: "paragraph", text: "New paragraph" },
                  ],
                })
              }
              className="px-2.5 py-1.5 text-xs border border-gray-300 rounded hover:bg-gray-50 disabled:opacity-50"
            >
              Add paragraph
            </button>
            <button
              type="button"
              disabled={disabled}
              onClick={() =>
                updateSection(sectionIndex, {
                  ...section,
                  blocks: [
                    ...section.blocks,
                    { kind: "bullet_list", items: ["New bullet"] }
                  ],
                })
              }
              className="px-2.5 py-1.5 text-xs border border-gray-300 rounded hover:bg-gray-50 disabled:opacity-50"
            >
              Add bullet list
            </button>
          </div>
        </section>
      ))}

      <button
        type="button"
        disabled={disabled}
        onClick={() =>
          onChange({ ...edit, sections: [...edit.sections, newReportSection()] })
        }
        className="w-full py-2 text-sm font-medium border border-dashed border-gray-400 text-gray-600 rounded-lg hover:bg-gray-50 disabled:opacity-50"
      >
        Add section
      </button>
    </div>
  );
}

function ReorderButtons({
  label,
  index,
  length,
  disabled,
  onMove,
}: {
  label: string;
  index: number;
  length: number;
  disabled: boolean;
  onMove: (to: number) => void;
}) {
  return (
    <div className="flex flex-col">
      <button
        type="button"
        aria-label={`Move ${label} up`}
        onClick={() => onMove(index - 1)}
        disabled={disabled || index === 0}
        className="px-1.5 text-gray-500 hover:text-gray-900 disabled:opacity-25"
      >
        ↑
      </button>
      <button
        type="button"
        aria-label={`Move ${label} down`}
        onClick={() => onMove(index + 1)}
        disabled={disabled || index === length - 1}
        className="px-1.5 text-gray-500 hover:text-gray-900 disabled:opacity-25"
      >
        ↓
      </button>
    </div>
  );
}
