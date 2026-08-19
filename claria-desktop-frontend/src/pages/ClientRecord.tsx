import { useClientWorkspace } from "../lib/useClientWorkspace";
import ClientWorkspaceTabs from "../components/ClientWorkspaceTabs";
import ClientChat from "./ClientChat";
import ClientRecordSettings from "./ClientRecordSettings";
import RecordTab from "./RecordTab";
import Writing from "./Writing";
import type { Page, PreferencesWriterSection } from "../App";

/** Compose one client's Record, Chat, and Writing workspaces. */
export default function ClientRecord({
  navigate,
  clientId,
  clientName,
  onClientNameChanged,
  onManageWriterSection,
}: {
  navigate: (page: Page) => void;
  clientId: string;
  clientName: string;
  onClientNameChanged: (name: string) => void;
  onManageWriterSection: (section: PreferencesWriterSection) => void;
}) {
  const workspace = useClientWorkspace(
    () => navigate("clients"),
    onManageWriterSection
  );

  return (
    <ClientWorkspaceTabs
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
          clientId={clientId}
          onResumeChat={workspace.openChat}
          onResumeWriting={workspace.openWriting}
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
