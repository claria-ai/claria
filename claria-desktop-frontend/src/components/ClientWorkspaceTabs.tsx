import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { BackButton } from "./icons";

export type ClientWorkspaceTab = "record" | "chat" | "with_tools";

const tabs: Array<{
  id: ClientWorkspaceTab;
  label: string;
  dataTab: string;
}> = [
  { id: "record", label: "Record", dataTab: "record" },
  { id: "chat", label: "Chat", dataTab: "chat" },
  { id: "with_tools", label: "With Tools", dataTab: "with-tools" },
];

export default function ClientWorkspaceTabs({
  clientName,
  activeTab,
  onSelect,
  onBack,
  children,
}: {
  clientName: string;
  activeTab: ClientWorkspaceTab;
  onSelect: (tab: ClientWorkspaceTab) => boolean;
  onBack: () => void;
  children: ReactNode;
}) {
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);

  function handleKeyDown(
    event: KeyboardEvent<HTMLButtonElement>,
    index: number
  ) {
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight") {
      nextIndex = (index + 1) % tabs.length;
    } else if (event.key === "ArrowLeft") {
      nextIndex = (index - 1 + tabs.length) % tabs.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = tabs.length - 1;
    }
    if (nextIndex === null) return;
    event.preventDefault();
    const next = tabs[nextIndex];
    if (onSelect(next.id)) {
      buttonRefs.current[nextIndex]?.focus();
    } else {
      const activeIndex = tabs.findIndex((tab) => tab.id === activeTab);
      buttonRefs.current[activeIndex]?.focus();
    }
  }

  return (
    <div className="flex flex-col h-screen">
      <div className="flex items-center gap-3 px-6 py-4 border-b border-gray-200 bg-white">
        <BackButton onClick={onBack} />
        <h2 className="text-lg font-semibold flex-1">{clientName}</h2>
        <div
          role="tablist"
          aria-label={`${clientName} workspace`}
          className="flex border border-gray-200 rounded-lg overflow-hidden"
        >
          {tabs.map((tab, index) => {
            const selected = activeTab === tab.id;
            return (
              <button
                key={tab.id}
                ref={(element) => {
                  buttonRefs.current[index] = element;
                }}
                id={`client-workspace-tab-${tab.id}`}
                role="tab"
                type="button"
                data-tab={tab.dataTab}
                aria-selected={selected}
                aria-controls={`client-workspace-panel-${tab.id}`}
                tabIndex={selected ? 0 : -1}
                onClick={() => {
                  if (!onSelect(tab.id)) {
                    const activeIndex = tabs.findIndex(
                      (candidate) => candidate.id === activeTab
                    );
                    buttonRefs.current[activeIndex]?.focus();
                  }
                }}
                onKeyDown={(event) => handleKeyDown(event, index)}
                className={`px-4 py-1.5 text-sm font-medium transition-colors ${
                  selected
                    ? "bg-blue-600 text-white"
                    : "bg-white text-gray-600 hover:bg-gray-50"
                }`}
              >
                {tab.label}
              </button>
            );
          })}
        </div>
      </div>
      <div
        id={`client-workspace-panel-${activeTab}`}
        role="tabpanel"
        aria-labelledby={`client-workspace-tab-${activeTab}`}
        className="flex-1 min-h-0 flex flex-col"
      >
        {children}
      </div>
    </div>
  );
}
