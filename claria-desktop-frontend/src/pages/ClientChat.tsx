import { useState, useEffect, useCallback, useRef } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  chatMessage,
  countClientContextTokens,
  extractRecordFile,
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
  const [contextError, setContextError] = useState<string | null>(null);
  const [previewContext, setPreviewContext] = useState<RecordContext | null>(
    null
  );
  const [extractingFile, setExtractingFile] = useState<string | null>(null);

  // Token count state
  const [contextTokens, setContextTokens] = useState<number | null>(null);
  const [countingTokens, setCountingTokens] = useState(false);
  const [tokenCountError, setTokenCountError] = useState<string | null>(null);

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
      .catch((e) => setContextError(String(e)))
      .finally(() => setContextLoading(false));
  }, [clientId]);

  // Count context tokens once context is loaded and models are available.
  // Only count files that have extracted text.
  useEffect(() => {
    if (contextLoading || chatModels.length === 0) return;
    const withText = contextFiles.filter((f) => f.text.length > 0);
    if (withText.length === 0) {
      setContextTokens(null);
      return;
    }
    setCountingTokens(true);
    setContextTokens(null);
    setTokenCountError(null);
    const filenames = withText.map((f) => f.filename);
    countClientContextTokens(clientId, chatModels[0].model_id, filenames)
      .then(setContextTokens)
      .catch((e) => setTokenCountError(String(e)))
      .finally(() => setCountingTokens(false));
  }, [contextLoading, contextFiles, chatModels, clientId]);

  function handleRemoveContext(filename: string) {
    setContextFiles((prev) => prev.filter((f) => f.filename !== filename));
  }

  async function handleExtract(filename: string) {
    setExtractingFile(filename);
    try {
      const updated = await extractRecordFile(clientId, filename);
      setContextFiles((prev) =>
        prev.map((f) => (f.filename === filename ? updated : f))
      );
    } catch (e) {
      // Show error briefly — the pill stays dimmed so the user can retry.
      alert(`Extraction failed: ${String(e)}`);
    } finally {
      setExtractingFile(null);
    }
  }

  // Resume a previous chat session when resumeChat prop is set.
  useEffect(() => {
    if (!resumeChat) return;
    setInitialMessages(resumeChat.messages);
    setInitialModelId(resumeChat.modelId);
    chatIdRef.current = resumeChat.chatId;
    onResumeChatConsumed?.();
  }, [resumeChat, onResumeChatConsumed]);

  const contextFilesRef = useRef(contextFiles);
  contextFilesRef.current = contextFiles;

  const handleSend = useCallback(
    async (modelId: string, messages: ChatMessage[]): Promise<string> => {
      const filenames = contextFilesRef.current
        .filter((f) => f.text.length > 0)
        .map((f) => f.filename);
      const response = await chatMessage(
        clientId,
        modelId,
        messages,
        chatIdRef.current,
        filenames
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
      {!contextLoading && contextError && (
        <div className="flex items-center gap-2 px-6 py-2 border-b border-red-100 bg-red-50">
          <span className="text-xs text-red-600">Failed to load context: {contextError}</span>
        </div>
      )}
      {!contextLoading && contextFiles.length > 0 && (
        <div className="flex items-center gap-2 px-6 py-2 border-b border-gray-100 bg-white flex-wrap">
          <span className="text-xs text-gray-400 shrink-0 inline-flex items-center gap-1">Context <TokenCountBadge counting={countingTokens} tokens={contextTokens} error={tokenCountError} />:</span>
          {contextFiles.map((cf) => {
            const hasText = cf.text.length > 0;
            const isExtracting = extractingFile === cf.filename;
            return (
              <span
                key={cf.filename}
                className={`shrink-0 inline-flex items-center gap-1 px-2.5 py-1 text-xs font-medium rounded-full ${
                  hasText
                    ? "text-blue-700 bg-blue-50 border border-blue-200"
                    : "text-gray-400 bg-gray-50 border border-gray-200"
                }`}
              >
                <button
                  onClick={() => hasText ? setPreviewContext(cf) : undefined}
                  className={hasText ? "hover:text-blue-900 transition-colors" : "cursor-default"}
                  title={hasText ? undefined : "No extracted text"}
                >
                  {cf.filename}
                </button>
                {!hasText && (
                  <button
                    onClick={() => handleExtract(cf.filename)}
                    disabled={isExtracting}
                    className="text-gray-400 hover:text-gray-600 transition-colors ml-0.5"
                    title="Extract text"
                  >
                    {isExtracting ? (
                      <svg className="w-3 h-3 animate-spin" viewBox="0 0 24 24" fill="none">
                        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                      </svg>
                    ) : (
                      <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                      </svg>
                    )}
                  </button>
                )}
                <button
                  onClick={() => handleRemoveContext(cf.filename)}
                  className={`${hasText ? "text-blue-400 hover:text-blue-700" : "text-gray-300 hover:text-gray-500"} transition-colors ml-0.5`}
                  title="Remove from context"
                >
                  <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </span>
            );
          })}
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

function TokenCountBadge({
  counting,
  tokens,
  error,
}: {
  counting: boolean;
  tokens: number | null;
  error?: string | null;
}) {
  const label =
    tokens != null
      ? tokens >= 1000
        ? `~${(tokens / 1000).toFixed(1)}k tokens`
        : `~${tokens} tokens`
      : null;

  if (counting) {
    return (
      <span className="shrink-0 inline-flex items-center justify-center w-5 h-5 text-gray-400">
        <svg
          className="animate-spin h-3.5 w-3.5"
          viewBox="0 0 24 24"
          fill="none"
        >
          <circle
            className="opacity-25"
            cx="12"
            cy="12"
            r="10"
            stroke="currentColor"
            strokeWidth="4"
          />
          <path
            className="opacity-75"
            fill="currentColor"
            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
          />
        </svg>
      </span>
    );
  }

  if (error) {
    return (
      <span
        className="shrink-0 inline-flex items-center justify-center w-5 h-5 rounded-full bg-red-50 border border-red-200 text-red-400 text-[10px] font-bold cursor-default group relative"
        title={error}
      >
        !
        <span className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 text-[11px] font-normal text-white bg-red-700 rounded max-w-xs whitespace-pre-wrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
          {error}
        </span>
      </span>
    );
  }

  if (label == null) return null;

  return (
    <span
      className="shrink-0 inline-flex items-center justify-center w-5 h-5 rounded-full bg-gray-100 border border-gray-200 text-gray-400 text-[10px] font-bold cursor-default group relative"
      title={label}
    >
      ?
      <span className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 text-[11px] font-normal text-white bg-gray-800 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
        {label}
      </span>
    </span>
  );
}
