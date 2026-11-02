import { useClientWorkspace } from "../lib/useClientWorkspace";
import ClientWorkspaceTabs from "../components/ClientWorkspaceTabs";
import ClientChat from "./ClientChat";
import ClientRecordSettings from "./ClientRecordSettings";
import RecordTab from "./RecordTab";
import Writing from "./Writing";
import type { Page, PreferencesFocusSection } from "../App";

/** Compose one client's Record, Chat, and Writing workspaces. */
export default function ClientRecord({
  navigate,
  clientId,
  clientName,
  onClientNameChanged,
  onManageWriterSection,
  luciaControlsInitiallyOpen = false,
  luciaPreviewsEnabled = false,
}: {
  navigate: (page: Page) => void;
  clientId: string;
  clientName: string;
  onClientNameChanged: (name: string) => void;
  onManageWriterSection: (section: PreferencesFocusSection) => void;
  luciaControlsInitiallyOpen?: boolean;
  luciaPreviewsEnabled?: boolean;
}) {
  const workspace = useClientWorkspace(
    () => navigate("clients"),
    onManageWriterSection
  );

  return (
    <ClientWorkspaceTabs
      clientId={clientId}
      luciaControlsInitiallyOpen={luciaControlsInitiallyOpen}
      luciaPreviewsEnabled={luciaPreviewsEnabled}
      clientName={clientName}
      activeView={workspace.activeView}
      onSelect={workspace.selectTab}
      onSettings={workspace.openSettings}
      onBack={workspace.back}
    >
      {workspace.activeView === "settings" ? (
        <ClientRecordSettings
          clientId={clientId}
          initialName={clientName}
          onNameChanged={onClientNameChanged}
        />
      ) : workspace.activeView === "record" ? (
        <RecordTab
          key={clientId}
          clientId={clientId}
          onResumeChat={workspace.openChat}
          onResumeWriting={workspace.openWriting}
          onChooseExtractionModel={() => workspace.manageWriterSection("extraction-model")}
        />
      ) : workspace.activeView === "chat" ? (
        <ClientChat
          key={workspace.pendingChat?.chatId ?? "new"}
          navigate={navigate}
          clientId={clientId}
          resumeChat={workspace.pendingChat}
        />
      ) : (
        <Writing
          key={`${clientId}:${workspace.expectedReportId ?? "current"}:${workspace.writingInstance}`}
          clientId={clientId}
          expectedReportId={workspace.expectedReportId}
          onLeaveStateChange={workspace.updateWritingLeaveState}
          onManageTemplates={() => workspace.manageWriterSection("writer-templates")}
          onManagePrompts={() => workspace.manageWriterSection("writer-prompts")}
        />
      )}
    </ClientWorkspaceTabs>
  );
}
