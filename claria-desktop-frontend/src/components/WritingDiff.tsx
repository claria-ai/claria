import type { ReactNode } from "react";
import { InlineMarkdown, MarkdownBlock } from "./Markdown";
import type { ReportBlock, ReportSection } from "../lib/tauri";

export type TableBlock = Extract<ReportBlock, { kind: "table" }>;

export function Change({
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

export function Comparison({
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

export function SectionPreview({
  section,
  tone = "proposed",
}: {
  section: ReportSection;
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

export function Blocks({ blocks }: { blocks: ReportBlock[] }) {
  return (
    <div className="border border-gray-200 rounded p-2 bg-white mt-1.5 space-y-1 text-xs leading-5 text-gray-700">
      {blocks.map((block, index) => {
        if (block.kind === "paragraph") {
          return <MarkdownBlock key={index} source={block.text} variant="xs" />;
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

export function TablePreview({
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

export function EmptyPreview() {
  return (
    <div className="border border-dashed border-gray-200 rounded p-2 text-xs italic text-gray-400">
      Nothing
    </div>
  );
}

export function PlainText({ text }: { text: string }) {
  return (
    <div className="text-xs text-gray-700 border border-gray-200 rounded p-2 bg-white">
      <MarkdownBlock source={text} variant="xs" />
    </div>
  );
}
