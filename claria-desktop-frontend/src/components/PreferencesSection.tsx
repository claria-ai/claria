import type { ReactNode, SyntheticEvent } from "react";

/**
 * One Preferences accordion: bordered card, summary row with a title, an
 * optional inline annotation, and the rotating chevron.
 *
 * Uncontrolled by default (`defaultOpen` sets the initial state and the
 * native `<details>` handles toggling). Pass `open` + `onToggle` for a
 * controlled section.
 */
export default function PreferencesSection({
  title,
  summary,
  defaultOpen = false,
  open,
  onToggle,
  contentClassName = "border-t border-gray-100 p-4",
  className = "border border-gray-200 rounded-lg",
  summaryClassName = "flex items-center justify-between p-4 cursor-pointer list-none [&::-webkit-details-marker]:hidden",
  titleClassName = "font-medium text-gray-900",
  testId,
  children,
}: {
  title: string;
  /** Small annotation rendered beside the title (current value, status). */
  summary?: ReactNode;
  defaultOpen?: boolean;
  /** Controlled open state; combine with `onToggle`. */
  open?: boolean;
  onToggle?: (open: boolean) => void;
  contentClassName?: string;
  className?: string;
  summaryClassName?: string;
  titleClassName?: string;
  testId?: string;
  children: ReactNode;
}) {
  return (
    <details
      className={`${className} group`}
      open={open !== undefined ? open : defaultOpen}
      onToggle={
        onToggle
          ? (event: SyntheticEvent<HTMLDetailsElement>) =>
              onToggle(event.currentTarget.open)
          : undefined
      }
      data-testid={testId}
    >
      <summary className={summaryClassName}>
        <div className="flex items-center gap-2">
          <span className={titleClassName}>{title}</span>
          {summary}
        </div>
        <span className="shrink-0 text-gray-400 text-xs transition-transform group-open:rotate-90">
          &#9656;
        </span>
      </summary>
      <div className={contentClassName}>{children}</div>
    </details>
  );
}
