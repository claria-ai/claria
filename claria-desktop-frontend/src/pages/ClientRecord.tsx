import { useState } from "react";
import { type ChatHistoryDetail, type ChatModel } from "../lib/tauri";
import { BackButton } from "../components/icons";
import ClientChat from "./ClientChat";
import RecordTab from "./RecordTab";
import type { Page } from "../App";
import type { ResumeChat } from "./ClientChat";

type Tab = "record" | "chat";

/**
 * One client, under two tabs: their record and a chat about it.
 *
 * The tabs share nothing but the client — except a chat resumed from the
 * record's history folder, which arrives here as a whole conversation and
 * switches the view to the tab that can show it.
 */
export default function ClientRecord({
  navigate,
  clientId,
  clientName,
  chatModels,
  chatModelsLoading,
  chatModelsError,
  preferredModelId,
}: {
  navigate: (page: Page) => void;
  clientId: string;
  clientName: string;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId?: string | null;
}) {
  const [tab, setTab] = useState<Tab>("record");
  const [resumeChat, setResumeChat] = useState<ResumeChat | null>(null);

  function handleResumeChat(detail: ChatHistoryDetail) {
    setResumeChat({
      chatId: detail.chat_id,
      modelId: detail.model_id,
      messages: detail.messages.map((m) => ({
        role: m.role,
        content: m.content,
      })),
      usageByIndex: detail.messages.map((m) => m.usage),
      lastActivityIso: detail.created_at,
    });
    setTab("chat");
  }

  return (
    <div className="flex flex-col h-screen">
      {/* Header */}
      <div className="flex items-center gap-3 px-6 py-4 border-b border-gray-200 bg-white">
        <BackButton onClick={() => navigate("clients")} />
        <h2 className="text-lg font-semibold flex-1">{clientName}</h2>

        {/* Tabs */}
        <div className="flex border border-gray-200 rounded-lg overflow-hidden">
          <button
            data-tab="record"
            onClick={() => setTab("record")}
            className={`px-4 py-1.5 text-sm font-medium transition-colors ${
              tab === "record"
                ? "bg-blue-600 text-white"
                : "bg-white text-gray-600 hover:bg-gray-50"
            }`}
          >
            Record
          </button>
          <button
            data-tab="chat"
            onClick={() => setTab("chat")}
            className={`px-4 py-1.5 text-sm font-medium transition-colors ${
              tab === "chat"
                ? "bg-blue-600 text-white"
                : "bg-white text-gray-600 hover:bg-gray-50"
            }`}
          >
            Chat
          </button>
        </div>
      </div>

      {/* Tab content */}
      {tab === "record" ? (
        <RecordTab clientId={clientId} onResumeChat={handleResumeChat} />
      ) : (
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
      )}
    </div>
  );
}
