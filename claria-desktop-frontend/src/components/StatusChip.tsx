export type StatusChipTone =
  | "neutral"
  | "info"
  | "progress"
  | "success"
  | "danger"
  | "warning"
  | "muted";

/**
 * Complete class literals per tone. Tailwind reads source text, so these can
 * never be assembled from a colour name and a shade.
 */
const TONE_CLASSES: Record<StatusChipTone, string> = {
  neutral: "border-gray-200 bg-gray-50 text-gray-600",
  info: "border-blue-200 bg-blue-50 text-blue-700",
  progress: "border-blue-300 bg-blue-50 text-blue-800",
  success: "border-emerald-200 bg-emerald-50 text-emerald-700",
  danger: "border-red-200 bg-red-50 text-red-700",
  warning: "border-amber-200 bg-amber-50 text-amber-800",
  muted: "border-gray-200 bg-white text-gray-400",
};

/**
 * A small labelled state marker. Surface-agnostic: it carries no margins and
 * no positioning, so a heading row, a card header, and a list row can each
 * place it themselves.
 */
export default function StatusChip({
  tone,
  label,
  animated = false,
  className = "",
}: {
  tone: StatusChipTone;
  label: string;
  /** Pulse the leading dot while the state is still moving. */
  animated?: boolean;
  className?: string;
}) {
  return (
    <span
      data-testid="status-chip"
      data-tone={tone}
      className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium whitespace-nowrap ${TONE_CLASSES[tone]} ${className}`}
    >
      {animated && (
        <span
          aria-hidden="true"
          className="h-1.5 w-1.5 rounded-full bg-current animate-pulse"
        />
      )}
      {label}
    </span>
  );
}
