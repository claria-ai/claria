import { type ChatModel } from "../lib/tauri";
import { useClientWorkspace } from "../lib/useClientWorkspace";
import ClientWorkspaceTabs from "../components/ClientWorkspaceTabs";
import ClientChat from "./ClientChat";
import RecordTab from "./RecordTab";
import Writing from "./Writing";
import type { Page } from "../App";

/** Compose one client's Record, Chat, and Writing workspaces. */
export default function ClientRecord({
  navigate,
  clientId,
  clientName,
  chatModels,
  chatModelsLoading,
  chatModelsError,
  preferredModelId,
  onRetryChatModels,
}: {
  navigate: (page: Page) => void;
  clientId: string;
  clientName: string;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId?: string | null;
  onRetryChatModels?: () => void;
}) {
  const workspace = useClientWorkspace(() => navigate("clients"));

  return (
    <ClientWorkspaceTabs
      clientName={clientName}
      activeTab={workspace.activeTab}
      onSelect={workspace.selectTab}
      onBack={workspace.back}
    >
      {workspace.activeTab === "record" ? (
        <RecordTab
          clientId={clientId}
          onResumeChat={workspace.openChat}
          onResumeWriting={workspace.openWriting}
        />
      ) : workspace.activeTab === "chat" ? (
        <ClientChat
          navigate={navigate}
          clientId={clientId}
          clientName={clientName}
          embedded
          resumeChat={workspace.pendingChat}
          onResumeChatConsumed={workspace.clearPendingChat}
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
        />
      ) : (
        <Writing
          key={`${clientId}:${workspace.expectedReportId ?? "current"}:${workspace.writingInstance}`}
          clientId={clientId}
          expectedReportId={workspace.expectedReportId}
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
          onLeaveStateChange={workspace.updateWritingLeaveState}
          onRetryModels={onRetryChatModels}
        />
      )}
    </ClientWorkspaceTabs>
  );
}
