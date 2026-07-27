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
import ChatHistoryHeader from "../components/ChatHistoryHeader";
import { BackButton, CloseIcon } from "../components/icons";
import Spinner from "../components/Spinner";
import TokenCountBadge from "../components/TokenCountBadge";
import Modal from "../components/Modal";
import { summarizeHistory } from "../lib/cost";

export type ResumeChat = {
  chatId: string;
  modelId: string;
  messages: ChatMessage[];
  /// Per-turn token usage aligned with `messages` by index. `null` for
  /// user turns and for legacy assistant turns whose history pre-dates
  /// per-turn usage tracking.
  usageByIndex?: Array<import("../lib/tauri").TurnUsage | null>;
  /// ISO-8601 timestamp of the chat's last activity (for the resume
  /// header — "Last activity: yesterday 4:12pm" etc).
  lastActivityIso?: string | null;
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
  const [extractError, setExtractError] = useState<{
    filename: string;
    message: string;
  } | null>(null);

  // Token count state
  const [contextTokens, setContextTokens] = useState<number | null>(null);
  const [countingTokens, setCountingTokens] = useState(false);
  const [tokenCountError, setTokenCountError] = useState<string | null>(null);

  // Resume chat state to pass to ChatWidget
  const [initialMessages, setInitialMessages] = useState<
    ChatMessage[] | undefined
  >();
  const [initialModelId, setInitialModelId] = useState<string | undefined>();
  const [initialUsageByIndex, setInitialUsageByIndex] = useState<
    Array<import("../lib/tauri").TurnUsage | null> | undefined
  >();
  const [lastActivityIso, setLastActivityIso] = useState<string | null>(null);

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
    setExtractError(null);
    try {
      const updated = await extractRecordFile(clientId, filename);
      setContextFiles((prev) =>
        prev.map((f) => (f.filename === filename ? updated : f))
      );
    } catch (e) {
      // The pill stays dimmed so the user can retry from the same button.
      setExtractError({ filename, message: String(e) });
    } finally {
      setExtractingFile(null);
    }
  }

  // Resume a previous chat session when resumeChat prop is set.
  useEffect(() => {
    if (!resumeChat) return;
    setInitialMessages(resumeChat.messages);
    setInitialModelId(resumeChat.modelId);
    setInitialUsageByIndex(resumeChat.usageByIndex);
    setLastActivityIso(resumeChat.lastActivityIso ?? null);
    chatIdRef.current = resumeChat.chatId;
    onResumeChatConsumed?.();
  }, [resumeChat, onResumeChatConsumed]);

  // Memo'd summary of resumed history for the chat header.
  const historyHeader = initialMessages
    ? (() => {
        const summary = summarizeHistory(
          initialMessages.map((m, i) => ({
            role: m.role,
            usage: initialUsageByIndex?.[i] ?? null,
          })),
          lastActivityIso
        );
        return (
          <ChatHistoryHeader
            summary={summary}
            onSeeAccountSpend={() => navigate("cost-explorer")}
          />
        );
      })()
    : undefined;

  const contextFilesRef = useRef(contextFiles);
  contextFilesRef.current = contextFiles;

  const handleSend = useCallback(
    async (modelId: string, messages: ChatMessage[]) => {
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
      return { content: response.content, usage: response.usage };
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
      {extractError && (
        <div className="flex items-center gap-2 px-6 py-2 border-b border-red-100 bg-red-50">
          <span className="flex-1 text-xs text-red-600">
            Could not extract text from {extractError.filename}:{" "}
            {extractError.message}
          </span>
          <button
            onClick={() => setExtractError(null)}
            aria-label="Dismiss"
            className="shrink-0 text-red-400 hover:text-red-700 transition-colors"
          >
            <CloseIcon className="w-3 h-3" />
          </button>
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
                      <Spinner className="w-3 h-3" />
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
                  <CloseIcon className="w-3 h-3" />
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
          <BackButton onClick={() => navigate("clients")} />
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
        initialUsageByIndex={initialUsageByIndex}
        contextTokens={contextTokens}
        emptyStateTitle="Start the conversation."
        emptyStateSubtitle="The chat includes the context files shown above. Chat messages are saved separately and do not modify your client files."
        extraLoading={contextLoading}
        extraLoadingText="Building context..."
        toolbar={toolbar}
        historyHeader={historyHeader}
        embedded={embedded}
      />

      {/* System prompt modal (read-only) */}
      {showPromptModal && systemPrompt && (
        <Modal
          open
          onClose={() => setShowPromptModal(false)}
          title="System Prompt"
          className="max-w-2xl p-6 max-h-[80vh] flex flex-col"
        >
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
        </Modal>
      )}

      {/* Context file preview modal */}
      {previewContext && (
        <Modal
          open
          onClose={() => setPreviewContext(null)}
          title={previewContext.filename}
          className="max-w-2xl p-6 max-h-[80vh] flex flex-col"
        >
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
        </Modal>
      )}
    </div>
  );
}
