/** Animated agent activity indicator for long-running, multi-step work. */
export default function AgentThrobber({
  label,
  detail,
  className = "",
}: {
  label: string;
  detail?: string;
  className?: string;
}) {
  return (
    <div
      role="status"
      aria-live="polite"
      className={`flex items-center gap-3 rounded-lg border border-blue-200 bg-blue-50 px-3 py-2.5 text-blue-900 ${className}`}
    >
      <span aria-hidden="true" className="flex h-5 items-center gap-1">
        {[0, 1, 2].map((dot) => (
          <span
            key={dot}
            className="h-1.5 w-1.5 rounded-full bg-blue-500 animate-bounce motion-reduce:animate-pulse"
            style={{ animationDelay: `${dot * 120}ms` }}
          />
        ))}
      </span>
      <span className="min-w-0">
        <span className="block text-xs font-semibold">{label}</span>
        {detail && (
          <span className="block truncate text-[11px] text-blue-700">{detail}</span>
        )}
      </span>
    </div>
  );
}
