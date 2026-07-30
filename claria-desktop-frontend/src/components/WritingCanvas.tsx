import { useEffect, useRef, useState, type ElementType } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type {
  ReportBlockView,
  ReportDraftEdit,
  ReportSectionEdit,
  ReportWorkspaceView,
} from "../lib/tauri";
import { dismissNotice, isNoticeDismissed } from "../lib/localPreference";
import { moveItem, newReportSection } from "../lib/writingWorkspace";
import { CloseIcon } from "./icons";

const EXPORT_NOTICE_KEY = "claria.writing.hide_export_notice";

export type WritingParagraphReference = {
  sectionId: string;
  blockIndex: number;
  sectionHeading: string;
  preview: string;
};

export default function WritingCanvas({
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
  onReference,
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
  onReference: (reference: WritingParagraphReference) => void;
  saveStatus: string | null;
  exportStatus: string | null;
  validationErrors: string[];
}) {
  const pending = workspace.pending_proposal !== null;
  const [showExportNotice, setShowExportNotice] = useState(
    () => !isNoticeDismissed(EXPORT_NOTICE_KEY)
  );

  function dismissExportNotice() {
    dismissNotice(EXPORT_NOTICE_KEY);
    setShowExportNotice(false);
  }

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
        {showExportNotice && (
          <div className="relative text-[11px] leading-4 text-amber-800 bg-amber-50 border border-amber-200 rounded px-3 py-2 pr-9">
            <p>
              Local exports may contain PHI and are not encrypted or managed by
              Claria. Store and share the .docx according to your organization&apos;s
              privacy and security requirements.
            </p>
            <button
              type="button"
              aria-label="Hide local export notice"
              title="Hide this notice"
              onClick={dismissExportNotice}
              className="absolute right-2 top-2 text-amber-600 hover:text-amber-900"
            >
              <CloseIcon className="w-3.5 h-3.5" />
            </button>
          </div>
        )}
        {(saveStatus || exportStatus || persistedExportStatus) && (
          <p role="status" aria-live="polite" className="text-xs text-gray-600">
            {saveStatus ?? exportStatus ?? persistedExportStatus}
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
            <AcceptedReport workspace={workspace} onReference={onReference} />
          )}
        </div>
      </div>
    </section>
  );
}

function AcceptedReport({
  workspace,
  onReference,
}: {
  workspace: ReportWorkspaceView;
  onReference: (reference: WritingParagraphReference) => void;
}) {
  const content = workspace.draft.content;
  return (
    <article data-testid="accepted-report-canvas">
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
                {section.blocks.map((block, blockIndex) =>
                  block.kind === "paragraph" ? (
                    <ParagraphDisplay
                      key={blockIndex}
                      text={block.text}
                      referenceLabel={`Reference ${section.heading}, paragraph ${blockIndex + 1} in Writing chat`}
                      onReference={() =>
                        onReference({
                          sectionId: section.id,
                          blockIndex,
                          sectionHeading: section.heading,
                          preview: referencePreview(block.text),
                        })
                      }
                    />
                  ) : (
                    <ul key={blockIndex} className="list-disc pl-6 space-y-1">
                      {block.items.map((item, itemIndex) => (
                        <li key={itemIndex}>
                          <MarkdownContent text={item} compact />
                        </li>
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

function ParagraphDisplay({
  text,
  referenceLabel,
  onReference,
}: {
  text: string;
  referenceLabel: string;
  onReference: () => void;
}) {
  return (
    <div className="group/paragraph relative rounded px-1 -mx-1 hover:bg-blue-50/40 focus-within:bg-blue-50/40">
      <MarkdownContent text={text} />
      <button
        type="button"
        aria-label={referenceLabel}
        title="Reference this paragraph in Writing chat"
        onClick={onReference}
        className="absolute -right-8 top-0 opacity-0 group-hover/paragraph:opacity-100 group-focus-within/paragraph:opacity-100 p-1 text-blue-500 hover:text-blue-800 bg-white border border-blue-200 rounded shadow-sm transition-opacity"
      >
        ↙
      </button>
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
    block: ReportBlockView
  ) => void;
  removeBlock: (sectionIndex: number, blockIndex: number) => void;
  onReference: (reference: WritingParagraphReference) => void;
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
                ) : (
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
                )}

                <div className="absolute -right-8 top-0 opacity-0 group-hover/block:opacity-100 group-focus-within/block:opacity-100 flex flex-col bg-white border border-gray-200 rounded shadow-sm transition-opacity">
                  {block.kind === "paragraph" && section.id && (
                    <button
                      type="button"
                      aria-label={`Reference ${section.heading}, paragraph ${blockIndex + 1} in Writing chat`}
                      title="Reference this paragraph in Writing chat"
                      onClick={() =>
                        onReference({
                          sectionId: section.id!,
                          blockIndex,
                          sectionHeading: section.heading,
                          preview: referencePreview(block.text),
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

function MarkdownContent({ text, compact = false }: { text: string; compact?: boolean }) {
  return (
    <div
      className={`prose prose-sm max-w-none prose-headings:my-2 prose-p:${compact ? "my-0" : "my-2"} prose-ul:my-2 prose-ol:my-2 prose-li:my-0 prose-code:text-inherit prose-code:before:content-none prose-code:after:content-none`}
    >
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

function referencePreview(text: string): string {
  const compact = text.replace(/\s+/g, " ").trim();
  return compact.length > 90 ? `${compact.slice(0, 87)}…` : compact;
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
