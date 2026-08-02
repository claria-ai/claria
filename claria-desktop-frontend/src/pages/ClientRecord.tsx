import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { type ChatHistoryDetail, type ChatModel } from "../lib/tauri";
import ClientWorkspaceTabs, {
  type ClientWorkspaceTab,
} from "../components/ClientWorkspaceTabs";
import ClientChat from "./ClientChat";
import RecordTab from "./RecordTab";
import Writing, { type WritingLeaveState } from "./Writing";
import type { Page } from "../App";
import type { ResumeChat } from "./ClientChat";

/**
 * One client's Record, Chat, and Writing workspaces.
 *
 * The workspace shell owns only cross-tab navigation: resuming persisted work,
 * retaining an in-memory chat, and guarding unsaved Writing changes. Each tab
 * owns its feature state and renders only while selected.
 */
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
  const [tab, setTab] = useState<ClientWorkspaceTab>("record");
  const [resumeChat, setResumeChat] = useState<ResumeChat | null>(null);
  const [writingLeaveState, setWritingLeaveState] = useState<WritingLeaveState>({
    hasUnsavedWork: false,
    hasUnsavedReportEdits: false,
    busy: false,
  });
  const [writingReportId, setWritingReportId] = useState<string | null>(null);
  const [writingKey, setWritingKey] = useState(0);

  function handleResumeChat(detail: ChatHistoryDetail) {
    setResumeChat({
      chatId: detail.chat_id,
      modelId: detail.model_id,
      messages: detail.messages.map((message) => ({
        role: message.role,
        content: message.content,
      })),
      usageByIndex: detail.messages.map((message) => message.usage),
      lastActivityIso: detail.created_at,
    });
    setTab("chat");
  }

  function handleResumeWriting(reportId: string) {
    setWritingReportId(reportId);
    setWritingKey((value) => value + 1);
    setTab("writing");
  }

  function mayLeaveWriting(): boolean {
    if (tab !== "writing") return true;
    if (writingLeaveState.busy) {
      window.alert("Wait for the current Writing action to finish before leaving.");
      return false;
    }
    // Composer text and report-block references are retained in memory while the
    // user navigates around Claria. Only inline report edits need confirmation.
    return (
      !writingLeaveState.hasUnsavedReportEdits ||
      window.confirm(
        "Discard unsaved report edits? Your typed Writing instruction and references will be kept."
      )
    );
  }

  function handleSelectTab(next: ClientWorkspaceTab): boolean {
    if (next === tab) return true;
    if (!mayLeaveWriting()) return false;
    if (next === "writing") {
      setWritingReportId(null);
      setWritingKey((value) => value + 1);
    }
    setTab(next);
    return true;
  }

  function handleBack() {
    if (mayLeaveWriting()) navigate("clients");
  }

  useEffect(() => {
    if (
      tab !== "writing" ||
      (!writingLeaveState.hasUnsavedWork && !writingLeaveState.busy)
    ) {
      return;
    }

    const beforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", beforeUnload);

    let disposed = false;
    let unlisten: (() => void) | undefined;
    if ("__TAURI_INTERNALS__" in window) {
      void getCurrentWindow()
        .onCloseRequested((event) => {
          if (writingLeaveState.busy) {
            event.preventDefault();
          } else if (
            writingLeaveState.hasUnsavedWork &&
            !window.confirm("Discard your unsaved report work and close Claria?")
          ) {
            event.preventDefault();
          }
        })
        .then((stopListening) => {
          if (disposed) stopListening();
          else unlisten = stopListening;
        });
    }

    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [writingLeaveState, tab]);

  return (
    <ClientWorkspaceTabs
      clientName={clientName}
      activeTab={tab}
      onSelect={handleSelectTab}
      onBack={handleBack}
    >
      {tab === "record" ? (
        <RecordTab
          clientId={clientId}
          onResumeChat={handleResumeChat}
          onResumeWriting={handleResumeWriting}
        />
      ) : tab === "chat" ? (
        <ClientChat
          navigate={navigate}
          clientId={clientId}
          clientName={clientName}
          embedded
          resumeChat={resumeChat}
          onResumeChatConsumed={() => setResumeChat(null)}
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
        />
      ) : (
        <Writing
          key={`${clientId}:${writingReportId ?? "current"}:${writingKey}`}
          clientId={clientId}
          expectedReportId={writingReportId}
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
          onLeaveStateChange={setWritingLeaveState}
          onRetryModels={onRetryChatModels}
        />
      )}
    </ClientWorkspaceTabs>
  );
}
