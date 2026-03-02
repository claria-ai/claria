import { useState, useEffect, useCallback, useRef } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  chatMessage,
  getPrompt,
  listRecordContext,
  type ChatMessage,
  type ChatModel,
  type RecordContext,
} from "../lib/tauri";
import type { Page } from "../App";
import ChatWidget from "../components/ChatWidget";

export type ResumeChat = {
  chatId: string;
  modelId: string;
  messages: ChatMessage[];
};

export default function ClientChat({
  navigate,
  clientId,
  clientName,
  embedded,
  resumeChat,
  onResumeChatConsumed,
  chatModels,
  chatModelsLoading,
  chatModelsError,
  preferredModelId,
}: {
  navigate: (page: Page) => void;
  clientId: string;
  clientName: string;
  embedded?: boolean;
  resumeChat?: ResumeChat | null;
  onResumeChatConsumed?: () => void;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId?: string | null;
}) {
  const chatIdRef = useRef<string | null>(null);

  // System prompt state
  const [systemPrompt, setSystemPrompt] = useState<string | null>(null);
  const [showPromptModal, setShowPromptModal] = useState(false);

  // Record context state
  const [contextFiles, setContextFiles] = useState<RecordContext[]>([]);
  const [contextLoading, setContextLoading] = useState(true);
  const [previewContext, setPreviewContext] = useState<RecordContext | null>(
    null
  );

  // Resume chat state to pass to ChatWidget
  const [initialMessages, setInitialMessages] = useState<
    ChatMessage[] | undefined
  >();
  const [initialModelId, setInitialModelId] = useState<string | undefined>();

  useEffect(() => {
    getPrompt("system-prompt")
      .then(setSystemPrompt)
      .catch(() => {});
    listRecordContext(clientId)
      .then(setContextFiles)
      .catch(() => {})
      .finally(() => setContextLoading(false));
  }, [clientId]);

  // Resume a previous chat session when resumeChat prop is set.
  useEffect(() => {
    if (!resumeChat) return;
    setInitialMessages(resumeChat.messages);
    setInitialModelId(resumeChat.modelId);
    chatIdRef.current = resumeChat.chatId;
    onResumeChatConsumed?.();
  }, [resumeChat, onResumeChatConsumed]);

  const handleSend = useCallback(
    async (modelId: string, messages: ChatMessage[]): Promise<string> => {
      const response = await chatMessage(
        clientId,
        modelId,
        messages,
        chatIdRef.current
      );
      chatIdRef.current = response.chat_id;
      return response.content;
    },
    [clientId]
  );

  const toolbar = (
    <>
      {embedded && systemPrompt && (
        <div className="flex items-center gap-2 px-6 py-1.5 border-b border-gray-100 bg-white">
          <button
            onClick={() => setShowPromptModal(true)}
            className="px-2.5 py-1 text-xs font-medium text-gray-500 bg-white border border-gray-200 rounded-full hover:bg-gray-100 transition-colors"
          >
            System Prompt
          </button>
        </div>
      )}
      {!contextLoading && contextFiles.length > 0 && (
        <div className="flex items-center gap-2 px-6 py-2 border-b border-gray-100 bg-white overflow-x-auto">
          <span className="text-xs text-gray-400 shrink-0">Context:</span>
          {contextFiles.map((cf) => (
            <button
              key={cf.filename}
              onClick={() => setPreviewContext(cf)}
              className="shrink-0 px-2.5 py-1 text-xs font-medium text-blue-700 bg-blue-50 border border-blue-200 rounded-full hover:bg-blue-100 transition-colors"
            >
              {cf.filename}
            </button>
          ))}
        </div>
      )}
    </>
  );

  return (
    <div className={`flex flex-col ${embedded ? "flex-1" : "h-screen"}`}>
      {/* Header — hidden when embedded in ClientRecord */}
      {!embedded && (
        <div className="flex items-center gap-3 px-6 py-4 border-b border-gray-200 bg-white">
          <button
            onClick={() => navigate("clients")}
            className="text-gray-500 hover:text-gray-700 transition-colors"
          >
            <svg
              className="w-5 h-5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M15 19l-7-7 7-7"
              />
            </svg>
          </button>
          <div className="flex-1">
            <h2 className="text-lg font-semibold">{clientName}</h2>
            <p className="text-xs text-gray-400">Chat</p>
          </div>
          {systemPrompt && (
            <button
              onClick={() => setShowPromptModal(true)}
              className="px-2.5 py-1 text-xs font-medium text-gray-500 bg-gray-100 rounded-full hover:bg-gray-200 transition-colors"
            >
              System Prompt
            </button>
          )}
        </div>
      )}

      <ChatWidget
        chatModels={chatModels}
        chatModelsLoading={chatModelsLoading}
        chatModelsError={chatModelsError}
        preferredModelId={preferredModelId}
        onSend={handleSend}
        initialMessages={initialMessages}
        initialModelId={initialModelId}
        emptyStateTitle="Start the conversation."
        emptyStateSubtitle="The chat includes the context files shown above. Chat messages are saved separately and do not modify your client files."
        extraLoading={contextLoading}
        extraLoadingText="Building context..."
        toolbar={toolbar}
        embedded={embedded}
      />

      {/* System prompt modal (read-only) */}
      {showPromptModal && systemPrompt && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold text-gray-900">
                System Prompt
              </h3>
              <button
                onClick={() => setShowPromptModal(false)}
                className="text-gray-400 hover:text-gray-600 transition-colors"
              >
                <svg
                  className="w-5 h-5"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M6 18L18 6M6 6l12 12"
                  />
                </svg>
              </button>
            </div>
            <div className="flex-1 overflow-y-auto border border-gray-200 rounded-lg p-4">
              <div className="prose prose-sm max-w-none">
                <Markdown remarkPlugins={[remarkGfm]}>{systemPrompt}</Markdown>
              </div>
            </div>
            <div className="flex justify-end mt-4">
              <button
                onClick={() => setShowPromptModal(false)}
                className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Context file preview modal */}
      {previewContext && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold text-gray-900">
                {previewContext.filename}
              </h3>
              <button
                onClick={() => setPreviewContext(null)}
                className="text-gray-400 hover:text-gray-600 transition-colors"
              >
                <svg
                  className="w-5 h-5"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M6 18L18 6M6 6l12 12"
                  />
                </svg>
              </button>
            </div>
            <div className="flex-1 overflow-y-auto border border-gray-200 rounded-lg p-4">
              <pre className="text-sm text-gray-700 whitespace-pre-wrap font-mono">
                {previewContext.text}
              </pre>
            </div>
            <div className="flex justify-end mt-4">
              <button
                onClick={() => setPreviewContext(null)}
                className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
