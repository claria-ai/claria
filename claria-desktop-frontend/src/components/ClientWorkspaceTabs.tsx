import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { BackButton, GearIcon } from "./icons";

export type ClientWorkspaceTab = "record" | "chat" | "writing";
export type ClientWorkspaceView = ClientWorkspaceTab | "settings";

const tabs: Array<{
  id: ClientWorkspaceTab;
  label: string;
  dataTab: string;
}> = [
  { id: "record", label: "Record", dataTab: "record" },
  { id: "chat", label: "Chat", dataTab: "chat" },
  { id: "writing", label: "Writing", dataTab: "writing" },
];

export default function ClientWorkspaceTabs({
  clientName,
  activeView,
  onSelect,
  onSettings,
  onBack,
  children,
}: {
  clientName: string;
  activeView: ClientWorkspaceView;
  onSelect: (tab: ClientWorkspaceTab) => boolean;
  onSettings: () => boolean;
  onBack: () => void;
  children: ReactNode;
}) {
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const settingsButtonRef = useRef<HTMLButtonElement | null>(null);

  function restoreActiveFocus() {
    const activeIndex = tabs.findIndex((tab) => tab.id === activeView);
    if (activeIndex >= 0) buttonRefs.current[activeIndex]?.focus();
    else settingsButtonRef.current?.focus();
  }

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
    if (onSelect(next.id)) buttonRefs.current[nextIndex]?.focus();
    else restoreActiveFocus();
  }

  const settingsActive = activeView === "settings";
  const panelId = settingsActive
    ? "client-workspace-settings"
    : `client-workspace-panel-${activeView}`;
  const panelLabel = settingsActive
    ? "client-workspace-settings-button"
    : `client-workspace-tab-${activeView}`;

  return (
    <div className="flex flex-col h-screen">
      <div className="flex items-center gap-3 px-6 py-4 border-b border-gray-200 bg-white">
        <BackButton onClick={onBack} />
        <button
          ref={settingsButtonRef}
          id="client-workspace-settings-button"
          type="button"
          data-view="settings"
          aria-label="Record settings"
          aria-pressed={settingsActive}
          title="Record settings"
          onClick={() => {
            if (!onSettings()) restoreActiveFocus();
          }}
          className={`p-1.5 rounded-md transition-colors ${
            settingsActive
              ? "bg-blue-100 text-blue-700"
              : "text-gray-400 hover:text-gray-700 hover:bg-gray-100"
          }`}
        >
          <GearIcon />
        </button>
        <h2 className="text-lg font-semibold flex-1">{clientName}</h2>
        <div
          role="tablist"
          aria-label={`${clientName} workspace`}
          className="flex border border-gray-200 rounded-lg overflow-hidden"
        >
          {tabs.map((tab, index) => {
            const selected = activeView === tab.id;
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
                tabIndex={selected || (settingsActive && index === 0) ? 0 : -1}
                onClick={() => {
                  if (!onSelect(tab.id)) restoreActiveFocus();
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
        id={panelId}
        role={settingsActive ? "region" : "tabpanel"}
        aria-labelledby={panelLabel}
        className="flex-1 min-h-0 flex flex-col"
      >
        {children}
      </div>
    </div>
  );
}
