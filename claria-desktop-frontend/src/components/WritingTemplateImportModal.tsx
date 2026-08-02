import { useState } from "react";
import type {
  ReportBlockView,
  ReportTemplatePreview,
} from "../lib/tauri";
import Modal from "./Modal";

export default function WritingTemplateImportModal({
  preview,
  currentRevision,
  busy,
  error,
  onCancel,
  onApply,
}: {
  preview: ReportTemplatePreview;
  currentRevision: number;
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onApply: () => void;
}) {
  const [acknowledged, setAcknowledged] = useState(false);

  return (
    <Modal
      open
      onClose={onCancel}
      title="Review DOCX template import"
      variant="framed"
      className="max-w-4xl max-h-[92vh] flex flex-col"
      dismissible={!busy}
    >
      <>
        <div className="flex-1 min-h-0 overflow-y-auto px-5 py-4 space-y-4">
            {error && (
              <p role="alert" className="rounded-md border border-red-300 bg-red-50 p-3 text-xs text-red-800">
                {error}
              </p>
            )}
            <div className="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-900">
              <p className="font-semibold">Treat completed reports as prior-client data</p>
              <p className="mt-1 text-xs leading-5">
                Claria cannot guarantee that rewriting names removes every old date,
                pronoun, diagnosis, score, or identifying fact. The source DOCX and
                its local path are not retained, but the imported report text becomes
                accepted client content if you continue and may remain in S3 version
                history after later rewrites. Prefer an approved blank template.
              </p>
            </div>

            <div className="grid grid-cols-2 sm:grid-cols-6 gap-2 text-center">
              <Stat label="Sections" value={preview.stats.sections} />
              <Stat label="Paragraphs" value={preview.stats.paragraphs} />
              <Stat label="Lists" value={preview.stats.bullet_lists} />
              <Stat label="Tables" value={preview.stats.tables} />
              <Stat label="Cells" value={preview.stats.table_cells} />
              <Stat label="Markers" value={preview.stats.placeholder_count} />
            </div>

            {preview.warnings.length > 0 && (
              <div className="rounded-md border border-amber-200 bg-amber-50 p-3">
                <p className="text-xs font-semibold text-amber-900">
                  Import notes
                </p>
                <ul className="mt-1 list-disc pl-5 text-xs leading-5 text-amber-800">
                  {preview.warnings.map((warning) => (
                    <li key={warning.code}>
                      {warning.message}
                      {warning.count > 1 ? ` (${warning.count})` : ""}
                    </li>
                  ))}
                </ul>
              </div>
            )}

            <div>
              <div className="flex items-end justify-between gap-3 mb-2">
                <div>
                  <p className="text-sm font-semibold text-gray-900">
                    Structured content preview
                  </p>
                  <p className="text-xs text-gray-500">
                    Claria imports headings, paragraphs, lists, and supported plain-text
                    tables. Export uses Claria&apos;s Word styling rather than the source
                    document&apos;s exact layout.
                  </p>
                </div>
                <p className="shrink-0 text-xs text-gray-500">
                  Replaces revision {currentRevision}
                </p>
              </div>
              <TemplateContentPreview preview={preview} />
            </div>

            <label className="flex items-start gap-2 rounded-md border border-gray-300 bg-gray-50 p-3 text-xs leading-5 text-gray-700">
              <input
                type="checkbox"
                checked={acknowledged}
                onChange={(event) => setAcknowledged(event.target.checked)}
                disabled={busy}
                className="mt-1"
              />
              <span>
                I reviewed the structured preview and understand that it will replace
                the visible accepted report as a new revision. I will verify all
                carried-over client facts before export.
              </span>
            </label>
          </div>
          <div className="shrink-0 border-t border-gray-200 px-5 py-4 flex justify-end gap-2">
            <button
              type="button"
              onClick={onCancel}
              disabled={busy}
              className="px-4 py-2 text-sm border border-gray-300 rounded-md bg-white hover:bg-gray-50 disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={onApply}
              disabled={busy || !acknowledged}
              className="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50"
            >
              {busy ? "Importing…" : "Import as accepted revision"}
            </button>
        </div>
      </>
    </Modal>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded border border-gray-200 bg-gray-50 px-2 py-2">
      <p className="text-base font-semibold text-gray-900">{value}</p>
      <p className="text-[10px] uppercase tracking-wide text-gray-500">{label}</p>
    </div>
  );
}

function TemplateContentPreview({ preview }: { preview: ReportTemplatePreview }) {
  return (
    <article className="max-h-[26rem] overflow-y-auto rounded border border-gray-300 bg-white p-6 shadow-inner select-text">
      <h1 className="text-2xl font-semibold text-center text-gray-900 mb-7">
        {preview.content.title}
      </h1>
      <div className="space-y-6">
        {preview.content.sections.map((section) => (
          <section key={section.id}>
            <h2 className="text-lg font-semibold text-gray-900 mb-2">
              {section.heading}
            </h2>
            <div className="space-y-2 text-sm leading-6 text-gray-700">
              {section.blocks.map((block, index) => (
                <TemplateBlock key={index} block={block} />
              ))}
            </div>
          </section>
        ))}
      </div>
    </article>
  );
}

function TemplateBlock({ block }: { block: ReportBlockView }) {
  if (block.kind === "paragraph") {
    return <p className="whitespace-pre-wrap">{block.text}</p>;
  }
  if (block.kind === "bullet_list") {
    return (
      <ul className="list-disc pl-6">
        {block.items.map((item, index) => (
          <li key={index} className="whitespace-pre-wrap">
            {item}
          </li>
        ))}
      </ul>
    );
  }
  return (
    <div className="overflow-x-auto rounded border border-gray-300">
      <table className="w-full border-collapse text-xs">
        {block.column_widths && (
          <colgroup>
            {block.column_widths.map((width, index) => (
              <col key={index} style={{ width: `${width / 100}%` }} />
            ))}
          </colgroup>
        )}
        <tbody>
          {block.rows.map((row, rowIndex) => (
            <tr
              key={rowIndex}
              className={block.has_header && rowIndex === 0 ? "bg-slate-100 font-semibold" : ""}
            >
              {row.map((cell, columnIndex) => (
                <td
                  key={columnIndex}
                  className="border-b border-r last:border-r-0 border-gray-200 px-2 py-1.5 whitespace-pre-wrap align-top"
                >
                  {cell || <span className="italic text-gray-400">blank</span>}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
