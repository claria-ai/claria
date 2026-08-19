import { useState, type ReactNode } from "react";
import { ChevronRightIcon, FolderIcon } from "./icons";

const ACCENTS = {
  purple: "bg-purple-100 text-purple-600",
  blue: "bg-blue-100 text-blue-600",
} as const;

/**
 * One collapsible folder row for persisted sessions (chat history, editor
 * history): folder icon, title, count line, chevron, and the caller's rows
 * when open.
 */
export default function HistoryFolder({
  title,
  subtitle,
  accent,
  children,
}: {
  title: string;
  /** The count line under the title, e.g. "3 conversations". */
  subtitle: string;
  accent: keyof typeof ACCENTS;
  /** Row content rendered when the folder is open. */
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);

  return (
    <div className="border-b border-gray-100">
      <button
        onClick={() => setOpen((value) => !value)}
        className="w-full px-4 py-3 flex items-center gap-3 hover:bg-gray-50 transition-colors"
      >
        <div
          className={`w-8 h-8 rounded flex items-center justify-center ${ACCENTS[accent]}`}
        >
          <FolderIcon />
        </div>
        <div className="flex-1 min-w-0 text-left">
          <p className="text-sm font-medium text-gray-900">{title}</p>
          <p className="text-xs text-gray-400">{subtitle}</p>
        </div>
        <ChevronRightIcon
          className={`w-4 h-4 text-gray-400 transition-transform ${
            open ? "rotate-90" : ""
          }`}
        />
      </button>
      {open && (
        <div className="divide-y divide-gray-100 bg-gray-50/50">{children}</div>
      )}
    </div>
  );
}
