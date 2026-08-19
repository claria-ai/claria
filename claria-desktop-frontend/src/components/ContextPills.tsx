import type { ContextPill } from "../lib/contextPills";

/** Writer context: report history, preloaded records, tool reads, and references. */
export default function ContextPills({
  pills,
  onPreviewFile,
}: {
  pills: ContextPill[];
  onPreviewFile: (filename: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5" aria-label="Writer context">
      {pills.map((pill) => {
        const className = `inline-flex max-w-full items-center gap-1 rounded-full border px-2 py-1 text-[10px] font-medium ${
          pill.status === "failed"
            ? "border-red-200 bg-red-50 text-red-700"
            : pill.status === "loading"
              ? "border-blue-200 bg-blue-50 text-blue-700"
              : "border-emerald-200 bg-emerald-50 text-emerald-700"
        }`;
        const content = (
          <>
            <span
              aria-hidden="true"
              className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                pill.status === "failed"
                  ? "bg-red-500"
                  : pill.status === "loading"
                    ? "bg-blue-500 animate-pulse"
                    : "bg-emerald-500"
              }`}
            />
            <span className="truncate">{pill.label}</span>
          </>
        );
        const previewHover =
          pill.status === "failed"
            ? "hover:border-red-400 hover:text-red-900"
            : pill.status === "loading"
              ? "hover:border-blue-400 hover:text-blue-900"
              : "hover:border-emerald-400 hover:text-emerald-900";
        return pill.filename ? (
          <button
            type="button"
            key={pill.key}
            onClick={() => onPreviewFile(pill.filename!)}
            title={`Preview ${pill.filename}`}
            className={`${className} ${previewHover}`}
          >
            {content}
          </button>
        ) : (
          <span key={pill.key} className={className}>
            {content}
          </span>
        );
      })}
    </div>
  );
}
