/**
 * Determinate progress bar. `valueText` is what a screen reader announces, and
 * by default it is also rendered under the bar; pass `showValueText={false}`
 * where an adjacent element already shows the same figure.
 */
export default function ProgressBar({
  value,
  max,
  label,
  valueText,
  showValueText = true,
  className = "",
}: {
  value: number;
  max: number;
  label: string;
  valueText: string;
  showValueText?: boolean;
  className?: string;
}) {
  const clamped = max > 0 ? Math.min(Math.max(value, 0), max) : 0;
  const percent = max > 0 ? Math.round((clamped / max) * 10000) / 100 : 0;

  return (
    <div className={className}>
      <div
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={max}
        aria-valuenow={clamped}
        aria-valuetext={valueText}
        className="h-1.5 bg-gray-200 rounded-full overflow-hidden"
      >
        <div
          className="h-full bg-blue-600 transition-[width]"
          style={{ width: `${percent}%` }}
        />
      </div>
      {showValueText && (
        <p className="mt-1 text-xs text-gray-600 tabular-nums">{valueText}</p>
      )}
    </div>
  );
}
