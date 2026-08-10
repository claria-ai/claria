import { memo, useEffect, useRef, type ElementType } from "react";
import { InlineMarkdown, MarkdownBlock } from "./Markdown";
import type {
  ReportBlock,
  ReportContent,
  ReportDraftEdit,
  ReportSectionEdit,
  ReportWorkspaceView,
} from "../lib/tauri";
import {
  moveItem,
  newReportSection,
  newReportTable,
} from "../lib/writingWorkspace";
import {
  reportBlockReferencePreview,
  type WritingBlockReference,
} from "../lib/writingComposerDraft";
import AgentThrobber from "./AgentThrobber";

/**
 * The accepted-report canvas. Memoized — the writer page re-renders on every
 * composer keystroke, and the canvas only depends on the workspace, the edit
 * buffer, and stable callbacks.
 */
export default memo(WritingCanvas);

function WritingCanvas({
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
  onOpenRevisions,
  onReference,
  status,
  validationErrors,
  agentActivity,
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
  onOpenRevisions: () => void;
  onReference: (reference: WritingBlockReference) => void;
  /** One transient status line (e.g. export progress); falls back to the
   *  persisted last-export line from the workspace. */
  status: string | null;
  validationErrors: string[];
  agentActivity?: { label: string; detail?: string } | null;
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
    block: ReportBlock
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

  const persistedExportStatus = workspace.last_export
    ? `${exportStatusLabel(workspace.last_export.status)} revision ${workspace.last_export.revision} · ${formatTimestamp(workspace.last_export.attempted_at)}`
    : null;

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
        <button
          type="button"
          onClick={onOpenRevisions}
          disabled={busy}
          className="px-3 py-1.5 text-xs font-medium border border-gray-300 rounded-md bg-white hover:bg-gray-50 disabled:opacity-50"
        >
          Revisions
        </button>
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
              Discard
            </button>
            <button
              type="button"
              onClick={onSave}
              disabled={busy || pending || !dirty || validationErrors.length > 0}
              className="px-3 py-1.5 text-xs font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50"
            >
              {busy ? "Working…" : "Save now"}
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
        {agentActivity && (
          <AgentThrobber
            label={agentActivity.label}
            detail={agentActivity.detail}
          />
        )}
        {(status || persistedExportStatus) && (
          <p role="status" aria-live="polite" className="text-xs text-gray-600">
            {status ?? persistedExportStatus}
          </p>
        )}
      </div>

      <div className="flex-1 overflow-y-auto p-6">
        <div className="max-w-3xl mx-auto bg-white min-h-full border border-gray-200 shadow-sm rounded-sm px-10 py-12 select-text">
          {editing ? (
            <>
              {validationErrors.length > 0 && (
                <div
                  role="alert"
                  className="mb-5 border border-red-200 bg-red-50 rounded-md p-3"
                >
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
                onReference={onReference}
              />
            </>
          ) : (
            <ReportDocument
              content={workspace.draft.content}
              onReference={onReference}
            />
          )}
        </div>
      </div>
    </section>
  );
}

export function ReportDocument({
  content,
  onReference,
  testId = "accepted-report-canvas",
}: {
  content: ReportContent;
  onReference?: (reference: WritingBlockReference) => void;
  testId?: string;
}) {
  return (
    <article data-testid={testId}>
      <h1 className="text-3xl font-semibold text-center text-gray-900 mb-10">
        <InlineMarkdown text={content.title} />
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
                <InlineMarkdown text={section.heading} />
              </h2>
              <div className="space-y-3 text-sm leading-6 text-gray-700">
                {section.blocks.map((block, blockIndex) => {
                  if (block.kind === "paragraph") {
                    return (
                      <ParagraphDisplay
                        key={blockIndex}
                        text={block.text}
                        referenceLabel={`Reference ${section.heading}, paragraph ${blockIndex + 1} in Writing chat`}
                        onReference={
                          onReference
                            ? () =>
                                onReference({
                                  kind: "paragraph",
                                  sectionId: section.id,
                                  blockIndex,
                                  sectionHeading: section.heading,
                                  preview: reportBlockReferencePreview(block),
                                })
                            : undefined
                        }
                      />
                    );
                  }
                  if (block.kind === "bullet_list") {
                    return (
                      <ul key={blockIndex} className="list-disc pl-6 space-y-1">
                        {block.items.map((item, itemIndex) => (
                          <li key={itemIndex}>
                            <MarkdownBlock source={item} variant="document-compact" />
                          </li>
                        ))}
                      </ul>
                    );
                  }
                  return (
                    <ReportTable
                      key={blockIndex}
                      table={block}
                      referenceLabel={`Reference ${section.heading}, table ${blockIndex + 1} in Writing chat`}
                      onReference={
                        onReference
                          ? () =>
                              onReference({
                                kind: "table",
                                sectionId: section.id,
                                blockIndex,
                                sectionHeading: section.heading,
                                preview: reportBlockReferencePreview(block),
                              })
                          : undefined
                      }
                    />
                  );
                })}
              </div>
            </section>
          ))}
        </div>
      )}
    </article>
  );
}

function ReportTable({
  table,
  referenceLabel,
  onReference,
}: {
  table: Extract<ReportBlock, { kind: "table" }>;
  referenceLabel: string;
  onReference?: () => void;
}) {
  const header = table.has_header ? table.rows[0] : null;
  const body = table.has_header ? table.rows.slice(1) : table.rows;
  return (
    <div className="group/table relative rounded hover:bg-blue-50/40 focus-within:bg-blue-50/40">
      <div
        className="overflow-x-auto rounded border border-gray-300"
        data-testid="report-table"
      >
        <table className="w-full border-collapse text-xs leading-5">
          <TableColumns table={table} />
          {header && (
            <thead className="bg-slate-100 text-gray-900">
              <tr>
                {header.map((cell, columnIndex) => (
                  <th
                    key={columnIndex}
                    scope="col"
                    className="border-b border-r last:border-r-0 border-gray-300 px-2 py-1.5 text-left font-semibold whitespace-pre-wrap align-top"
                  >
                    {cell}
                  </th>
                ))}
              </tr>
            </thead>
          )}
          <tbody>
            {body.map((row, rowIndex) => (
              <tr key={rowIndex} className="even:bg-gray-50/60">
                {row.map((cell, columnIndex) => (
                  <td
                    key={columnIndex}
                    className="border-b last:border-b-0 border-r last:border-r-0 border-gray-200 px-2 py-1.5 whitespace-pre-wrap align-top"
                  >
                    {cell}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {onReference && (
        <button
          type="button"
          aria-label={referenceLabel}
          title="Reference this table in Writing chat"
          onClick={onReference}
          className="absolute -right-8 top-0 opacity-0 group-hover/table:opacity-100 group-focus-within/table:opacity-100 p-1 text-blue-500 hover:text-blue-800 bg-white border border-blue-200 rounded shadow-sm transition-opacity"
        >
          ↙
        </button>
      )}
    </div>
  );
}

function TableColumns({
  table,
}: {
  table: Extract<ReportBlock, { kind: "table" }>;
}) {
  if (!table.column_widths) return null;
  return (
    <colgroup>
      {table.column_widths.map((width, index) => (
        <col key={index} style={{ width: `${width / 100}%` }} />
      ))}
    </colgroup>
  );
}

function ParagraphDisplay({
  text,
  referenceLabel,
  onReference,
}: {
  text: string;
  referenceLabel: string;
  onReference?: () => void;
}) {
  return (
    <div className="group/paragraph relative rounded px-1 -mx-1 hover:bg-blue-50/40 focus-within:bg-blue-50/40">
      <MarkdownBlock source={text} variant="document" />
      {onReference && (
        <button
          type="button"
          aria-label={referenceLabel}
          title="Reference this paragraph in Writing chat"
          onClick={onReference}
          className="absolute -right-8 top-0 opacity-0 group-hover/paragraph:opacity-100 group-focus-within/paragraph:opacity-100 p-1 text-blue-500 hover:text-blue-800 bg-white border border-blue-200 rounded shadow-sm transition-opacity"
        >
          ↙
        </button>
      )}
    </div>
  );
}

function EditableReport({
  edit,
  disabled,
  onChange,
  updateSection,
  updateBlock,
  removeBlock,
  onReference,
}: {
  edit: ReportDraftEdit;
  disabled: boolean;
  onChange: (edit: ReportDraftEdit) => void;
  updateSection: (index: number, section: ReportSectionEdit) => void;
  updateBlock: (
    sectionIndex: number,
    blockIndex: number,
    block: ReportBlock
  ) => void;
  removeBlock: (sectionIndex: number, blockIndex: number) => void;
  onReference: (reference: WritingBlockReference) => void;
}) {
  return (
    <div className="space-y-8" data-testid="inline-report-editor">
      <EditableText
        as="h1"
        ariaLabel="Report title"
        value={edit.title}
        onChange={(title) => onChange({ ...edit, title })}
        disabled={disabled}
        className="text-3xl font-semibold text-center text-gray-900 mb-10"
      />

      {edit.sections.map((section, sectionIndex) => (
        <section
          key={section.id ?? `new-${sectionIndex}`}
          className="group/section relative"
        >
          <EditableText
            as="h2"
            ariaLabel={`Section ${sectionIndex + 1} heading`}
            value={section.heading}
            onChange={(heading) =>
              updateSection(sectionIndex, { ...section, heading })
            }
            disabled={disabled}
            className="text-xl font-semibold text-gray-900 mb-3"
          />

          <div className="absolute -right-8 top-0 opacity-0 group-hover/section:opacity-100 group-focus-within/section:opacity-100 flex flex-col bg-white border border-gray-200 rounded shadow-sm">
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
              aria-label={`Remove section ${sectionIndex + 1}`}
              title="Remove section"
              onClick={() =>
                onChange({
                  ...edit,
                  sections: edit.sections.filter(
                    (_, index) => index !== sectionIndex
                  ),
                })
              }
              disabled={disabled}
              className="px-1.5 py-1 text-xs text-red-600 hover:bg-red-50 disabled:opacity-50"
            >
              ×
            </button>
          </div>

          <div className="space-y-3 text-sm leading-6 text-gray-700">
            {section.blocks.map((block, blockIndex) => (
              <div
                key={blockIndex}
                className="group/block relative rounded px-1 -mx-1 hover:bg-blue-50/40 focus-within:bg-blue-50/40"
              >
                {block.kind === "paragraph" ? (
                  <EditableText
                    as="p"
                    ariaLabel={`Section ${sectionIndex + 1} paragraph ${blockIndex + 1}`}
                    value={block.text}
                    onChange={(text) =>
                      updateBlock(sectionIndex, blockIndex, {
                        kind: "paragraph",
                        text,
                      })
                    }
                    disabled={disabled}
                    multiline
                  />
                ) : block.kind === "bullet_list" ? (
                  <ul className="list-disc pl-6 space-y-1">
                    {block.items.map((item, itemIndex) => (
                      <li key={itemIndex}>
                        <EditableText
                          as="span"
                          ariaLabel={`Section ${sectionIndex + 1} bullet ${itemIndex + 1}`}
                          value={item}
                          onChange={(value) => {
                            const items = [...block.items];
                            items[itemIndex] = value;
                            updateBlock(sectionIndex, blockIndex, {
                              kind: "bullet_list",
                              items,
                            });
                          }}
                          disabled={disabled}
                        />
                      </li>
                    ))}
                  </ul>
                ) : (
                  <EditableTable
                    table={block}
                    sectionIndex={sectionIndex}
                    blockIndex={blockIndex}
                    disabled={disabled}
                    onChange={(table) =>
                      updateBlock(sectionIndex, blockIndex, table)
                    }
                  />
                )}

                <div className="absolute -right-8 top-0 opacity-0 group-hover/block:opacity-100 group-focus-within/block:opacity-100 flex flex-col bg-white border border-gray-200 rounded shadow-sm transition-opacity">
                  {(block.kind === "paragraph" || block.kind === "table") &&
                    section.id && (
                      <button
                        type="button"
                        aria-label={`Reference ${section.heading}, ${block.kind === "paragraph" ? "paragraph" : "table"} ${blockIndex + 1} in Writing chat`}
                        title={`Reference this ${block.kind} in Writing chat`}
                        onClick={() =>
                          onReference({
                            kind: block.kind,
                            sectionId: section.id!,
                            blockIndex,
                            sectionHeading: section.heading,
                            preview: reportBlockReferencePreview(block),
                          })
                        }
                        disabled={disabled}
                        className="px-1.5 py-1 text-blue-600 hover:bg-blue-50 disabled:opacity-50"
                      >
                        ↙
                      </button>
                    )}
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
                    aria-label={`Remove block ${blockIndex + 1}`}
                    title="Remove block"
                    onClick={() => removeBlock(sectionIndex, blockIndex)}
                    disabled={disabled}
                    className="px-1.5 py-1 text-xs text-red-600 hover:bg-red-50 disabled:opacity-50"
                  >
                    ×
                  </button>
                </div>
              </div>
            ))}
          </div>

          <div className="mt-3 flex gap-2 opacity-0 group-hover/section:opacity-100 group-focus-within/section:opacity-100 transition-opacity">
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
              className="px-2.5 py-1.5 text-xs border border-gray-300 bg-white rounded hover:bg-gray-50 disabled:opacity-50"
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
                    { kind: "bullet_list", items: ["New bullet"] },
                  ],
                })
              }
              className="px-2.5 py-1.5 text-xs border border-gray-300 bg-white rounded hover:bg-gray-50 disabled:opacity-50"
            >
              Add bullet list
            </button>
            <button
              type="button"
              disabled={disabled}
              onClick={() =>
                updateSection(sectionIndex, {
                  ...section,
                  blocks: [...section.blocks, newReportTable()],
                })
              }
              className="px-2.5 py-1.5 text-xs border border-gray-300 bg-white rounded hover:bg-gray-50 disabled:opacity-50"
            >
              Add table
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

function EditableTable({
  table,
  sectionIndex,
  blockIndex,
  disabled,
  onChange,
}: {
  table: Extract<ReportBlock, { kind: "table" }>;
  sectionIndex: number;
  blockIndex: number;
  disabled: boolean;
  onChange: (table: Extract<ReportBlock, { kind: "table" }>) => void;
}) {
  const columns = table.rows[0]?.length ?? 0;

  function updateCell(rowIndex: number, columnIndex: number, value: string) {
    const rows = table.rows.map((row) => [...row]);
    rows[rowIndex][columnIndex] = value;
    onChange({ ...table, rows });
  }

  return (
    <div className="rounded border border-gray-300 bg-white overflow-hidden">
      <div className="flex flex-wrap items-center gap-2 border-b border-gray-200 bg-gray-50 px-2 py-1.5 text-[11px]">
        <label className="inline-flex items-center gap-1.5 text-gray-700">
          <input
            type="checkbox"
            checked={table.has_header}
            onChange={(event) =>
              onChange({ ...table, has_header: event.target.checked })
            }
            disabled={disabled}
          />
          First row is header
        </label>
        <button
          type="button"
          disabled={disabled || table.rows.length >= 200}
          onClick={() =>
            onChange({
              ...table,
              rows: [...table.rows, Array.from({ length: columns }, () => "")],
            })
          }
          className="ml-auto px-2 py-0.5 border border-gray-300 rounded bg-white hover:bg-gray-100 disabled:opacity-50"
        >
          Add row
        </button>
        <button
          type="button"
          disabled={disabled || columns >= 20}
          onClick={() =>
            onChange({
              ...table,
              rows: table.rows.map((row) => [...row, ""]),
              column_widths: null,
            })
          }
          className="px-2 py-0.5 border border-gray-300 rounded bg-white hover:bg-gray-100 disabled:opacity-50"
        >
          Add column
        </button>
        <button
          type="button"
          disabled={disabled || columns <= 1}
          onClick={() =>
            onChange({
              ...table,
              rows: table.rows.map((row) => row.slice(0, -1)),
              column_widths: null,
            })
          }
          className="px-2 py-0.5 border border-gray-300 rounded bg-white hover:bg-gray-100 disabled:opacity-50"
        >
          Remove last column
        </button>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full border-collapse text-xs leading-5">
          <TableColumns table={table} />
          <tbody>
            {table.rows.map((row, rowIndex) => (
              <tr
                key={rowIndex}
                className={
                  table.has_header && rowIndex === 0
                    ? "bg-slate-100 font-semibold"
                    : "even:bg-gray-50/60"
                }
              >
                {row.map((cell, columnIndex) => {
                  const Cell = table.has_header && rowIndex === 0 ? "th" : "td";
                  return (
                    <Cell
                      key={columnIndex}
                      scope={Cell === "th" ? "col" : undefined}
                      className="border-b border-r border-gray-200 px-2 py-1 align-top text-left min-w-24"
                    >
                      <EditableText
                        as="span"
                        ariaLabel={`Section ${sectionIndex + 1} table ${blockIndex + 1}, row ${rowIndex + 1}, column ${columnIndex + 1}`}
                        value={cell}
                        onChange={(value) =>
                          updateCell(rowIndex, columnIndex, value)
                        }
                        disabled={disabled}
                        multiline
                        className="block min-h-5"
                      />
                    </Cell>
                  );
                })}
                <td className="border-b border-gray-200 px-1 align-middle w-7">
                  <button
                    type="button"
                    aria-label={`Remove table row ${rowIndex + 1}`}
                    title="Remove row"
                    disabled={disabled || table.rows.length <= 1}
                    onClick={() =>
                      onChange({
                        ...table,
                        rows: table.rows.filter((_, index) => index !== rowIndex),
                      })
                    }
                    className="px-1 text-red-600 hover:bg-red-50 rounded disabled:opacity-25"
                  >
                    ×
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function EditableText({
  as: Component,
  ariaLabel,
  value,
  onChange,
  disabled,
  className,
  multiline = false,
}: {
  as: ElementType;
  ariaLabel: string;
  value: string;
  onChange: (value: string) => void;
  disabled: boolean;
  className?: string;
  multiline?: boolean;
}) {
  const ref = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const element = ref.current;
    if (element && document.activeElement !== element && element.innerText !== value) {
      element.innerText = value;
    }
  }, [value]);

  return (
    <Component
      ref={ref}
      role="textbox"
      aria-label={ariaLabel}
      aria-multiline={multiline || undefined}
      contentEditable={!disabled}
      suppressContentEditableWarning
      spellCheck
      onInput={(event: React.FormEvent<HTMLElement>) =>
        onChange(event.currentTarget.innerText.replace(/\u00a0/g, " "))
      }
      onKeyDown={(event: React.KeyboardEvent<HTMLElement>) => {
        if (!multiline && event.key === "Enter") event.preventDefault();
      }}
      className={`${className ?? ""} whitespace-pre-wrap rounded px-1 -mx-1 outline-none hover:bg-gray-50 focus:bg-white focus:ring-2 focus:ring-blue-400 ${
        disabled ? "opacity-60" : ""
      }`}
    />
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

function exportStatusLabel(status: "exported" | "canceled" | "failed"): string {
  if (status === "exported") return "Last exported";
  if (status === "failed") return "Last export failed at";
  return "Last export canceled at";
}

function formatTimestamp(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
