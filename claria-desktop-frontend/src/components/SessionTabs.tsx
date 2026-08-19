import { useRef, type KeyboardEvent, type ReactNode } from "react";

export type SessionTabOption<T extends string> = {
  id: T;
  label: string;
  /** Compact tabs render only their icon but retain `label` as their name. */
  icon?: ReactNode;
  compact?: boolean;
};

/**
 * Small, shared in-session tabs used by Chat and Writing. These deliberately
 * sit below the surface toolbar so navigation, model choice, and content keep
 * a consistent hierarchy on both screens.
 */
export default function SessionTabs<T extends string>({
  idPrefix,
  label,
  tabs,
  active,
  onSelect,
}: {
  idPrefix: string;
  label: string;
  tabs: ReadonlyArray<SessionTabOption<T>>;
  active: T;
  onSelect: (tab: T) => void;
}) {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);

  function handleKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight") nextIndex = (index + 1) % tabs.length;
    else if (event.key === "ArrowLeft") {
      nextIndex = (index - 1 + tabs.length) % tabs.length;
    } else if (event.key === "Home") nextIndex = 0;
    else if (event.key === "End") nextIndex = tabs.length - 1;
    if (nextIndex === null) return;
    event.preventDefault();
    onSelect(tabs[nextIndex].id);
    refs.current[nextIndex]?.focus();
  }

  return (
    <div
      role="tablist"
      aria-label={label}
      className="flex min-h-10 items-end gap-1 overflow-x-auto border-b border-gray-200 bg-white px-4"
    >
      {tabs.map((tab, index) => {
        const selected = tab.id === active;
        return (
          <button
            key={tab.id}
            ref={(element) => {
              refs.current[index] = element;
            }}
            id={`${idPrefix}-tab-${tab.id}`}
            type="button"
            role="tab"
            aria-label={tab.label}
            aria-selected={selected}
            aria-controls={`${idPrefix}-panel-${tab.id}`}
            title={tab.compact ? tab.label : undefined}
            tabIndex={selected ? 0 : -1}
            onClick={() => onSelect(tab.id)}
            onKeyDown={(event) => handleKeyDown(event, index)}
            className={`${
              tab.compact
                ? "ml-auto w-14 shrink-0 justify-center px-2"
                : "min-w-0 px-3"
            } inline-flex h-9 items-center whitespace-nowrap border-b-2 text-xs font-medium transition-colors ${
              selected
                ? "border-blue-600 text-blue-700"
                : "border-transparent text-gray-500 hover:border-gray-300 hover:text-gray-800"
            }`}
          >
            {tab.icon ?? tab.label}
          </button>
        );
      })}
    </div>
  );
}

/** Compact lightning-and-dollar glyph shared by every usage tab. */
export function UsageTabIcon() {
  return (
    <span aria-hidden="true" className="inline-flex items-center gap-1 leading-none">
      <svg
        className="h-3.5 w-3.5"
        viewBox="0 0 20 20"
        fill="currentColor"
      >
        <path d="M11.3 1.3 4.7 10h4.1l-.7 8.7 7.2-10H11l.3-7.4Z" />
      </svg>
      <span className="text-[9px] text-gray-300">/</span>
      <span className="text-[13px] font-semibold">$</span>
    </span>
  );
}
