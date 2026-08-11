import { useState, useCallback, useMemo, useRef } from "react";
import {
  chatMessage,
  countClientContextTokens,
  extractRecordFile,
  getPrompt,
  listRecordContext,
  renameChatHistory,
  type ChatMessage,
  type RecordContext,
} from "../lib/tauri";
import type { Page } from "../App";
import ChatWidget from "../components/ChatWidget";
import ChatHistoryHeader from "../components/ChatHistoryHeader";
import EditableName from "../components/EditableName";
import { CloseIcon } from "../components/icons";
import Spinner from "../components/Spinner";
import TextPreviewModal from "../components/TextPreviewModal";
import TokenCountBadge from "../components/TokenCountBadge";
import { summarizeHistory } from "../lib/cost";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import { useContextTokens } from "../lib/useContextTokens";

export type ResumeChat = {
  chatId: string;
  name: string;
  modelId: string;
  messages: ChatMessage[];
  /// Per-turn token usage aligned with `messages` by index. `null` for
  /// user turns and for legacy assistant turns whose history pre-dates
  /// per-turn usage tracking.
  usageByIndex?: Array<import("../lib/tauri").TurnUsage | null>;
  timestampsByIndex?: Array<string | null>;
  /// ISO-8601 timestamp of the chat's last activity (for the resume
  /// header — "Last activity: yesterday 4:12pm" etc).
  lastActivityIso?: string | null;
};

export default function ClientChat({
  navigate,
  clientId,
  resumeChat,
}: {
  navigate: (page: Page) => void;
  clientId: string;
  /// A persisted chat being resumed. Identity changes must remount this
  /// component — the caller keys on `resumeChat?.chatId`.
  resumeChat?: ResumeChat | null;
}) {
  const chatIdRef = useRef<string | null>(resumeChat?.chatId ?? null);
  const [chatName, setChatName] = useState(resumeChat?.name ?? "New chat");
  const [hasPersistedChat, setHasPersistedChat] = useState(resumeChat != null);

  // System prompt (read-only; missing prompt hides the button).
  const { data: systemPrompt } = useAsyncLoad(
    () => getPrompt("system-prompt"),
    []
  );
  const [showPromptModal, setShowPromptModal] = useState(false);

  // Record context: the fetched list plus local remove/extract overlays,
  // so user edits never mirror server data into duplicate state.
  const contextLoad = useAsyncLoad(() => listRecordContext(clientId), [clientId]);
  const contextLoading = contextLoad.loading;
  const contextError = contextLoad.error;
  const [removedContext, setRemovedContext] = useState<ReadonlySet<string>>(
    new Set()
  );
  const [extractedContext, setExtractedContext] = useState<
    ReadonlyMap<string, RecordContext>
  >(new Map());
  const contextFiles = useMemo(
    () =>
      (contextLoad.data ?? [])
        .filter((file) => !removedContext.has(file.filename))
        .map((file) => extractedContext.get(file.filename) ?? file),
    [contextLoad.data, removedContext, extractedContext]
  );
  const [previewContext, setPreviewContext] = useState<RecordContext | null>(
    null
  );
  const [extractingFile, setExtractingFile] = useState<string | null>(null);
  const [extractError, setExtractError] = useState<{
    filename: string;
    message: string;
  } | null>(null);

  // Resumed-chat snapshot — plain initial state, valid for this mount's
  // lifetime because the caller remounts on resume-identity change.
  const initialMessages = resumeChat?.messages;
  const initialModelId = resumeChat?.modelId;
  const initialUsageByIndex = resumeChat?.usageByIndex;
  const initialTimestampsByIndex = resumeChat?.timestampsByIndex;
  const lastActivityIso = resumeChat?.lastActivityIso ?? null;

  // Count context tokens once context is loaded. Only files with extracted
  // text are counted — the rest contribute nothing to the prompt.
  const contextFilenames = useMemo(
    () => contextFiles.filter((f) => f.text.length > 0).map((f) => f.filename),
    [contextFiles]
  );
  const countContext = useCallback(
    () => countClientContextTokens(clientId, contextFilenames),
    [clientId, contextFilenames]
  );
  const {
    tokens: contextTokens,
    counting: countingTokens,
    error: tokenCountError,
  } = useContextTokens(
    contextLoading || contextFilenames.length === 0 ? null : countContext
  );

  function handleRemoveContext(filename: string) {
    setRemovedContext((prev) => new Set(prev).add(filename));
  }

  async function handleExtract(filename: string) {
    setExtractingFile(filename);
    setExtractError(null);
    try {
      const updated = await extractRecordFile(clientId, filename);
      setExtractedContext((prev) => new Map(prev).set(filename, updated));
    } catch (e) {
      // The pill stays dimmed so the user can retry from the same button.
      setExtractError({ filename, message: String(e) });
    } finally {
      setExtractingFile(null);
    }
  }

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
    async (
      modelId: string,
      messages: ChatMessage[],
      onDelta: (text: string) => void
    ) => {
      const filenames = contextFilesRef.current
        .filter((f) => f.text.length > 0)
        .map((f) => f.filename);
      const response = await chatMessage(
        clientId,
        modelId,
        messages,
        chatIdRef.current,
        filenames,
        chatIdRef.current || chatName === "New chat" ? null : chatName,
        (event) => {
          if (event.kind === "delta") onDelta(event.text);
        }
      );
      chatIdRef.current = response.chat_id;
      setChatName(response.chat_name);
      setHasPersistedChat(true);
      return { content: response.content, usage: response.usage };
    },
    [clientId, chatName]
  );

  async function handleRename(name: string) {
    const chatId = chatIdRef.current;
    if (!chatId) {
      setChatName(name);
      return;
    }
    const updated = await renameChatHistory(clientId, chatId, name);
    setChatName(updated.name);
  }

  const toolbar = (
    <>
      <div className="flex items-center gap-3 px-6 py-2 border-b border-gray-100 bg-white">
        <EditableName
          value={chatName}
          label="chat"
          onSave={handleRename}
          className="flex-1"
        />
        {!hasPersistedChat && chatName === "New chat" && (
          <span className="text-[11px] text-gray-400">
            A numbered name is assigned after the first message.
          </span>
        )}
      </div>
      {systemPrompt && (
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
    <div className="flex flex-col flex-1">
      <ChatWidget
        onSend={handleSend}
        initialMessages={initialMessages}
        initialModelId={initialModelId}
        initialUsageByIndex={initialUsageByIndex}
        initialTimestampsByIndex={initialTimestampsByIndex}
        contextTokens={contextTokens}
        emptyStateTitle="Start the conversation."
        emptyStateSubtitle="The chat includes the context files shown above. Chat messages are saved separately and do not modify your client files."
        extraLoading={contextLoading}
        extraLoadingText="Building context..."
        toolbar={toolbar}
        historyHeader={historyHeader}
      />

      {/* System prompt modal (read-only) */}
      {showPromptModal && systemPrompt && (
        <TextPreviewModal
          filename="System Prompt"
          text={systemPrompt}
          markdown
          onClose={() => setShowPromptModal(false)}
        />
      )}

      {/* Context file preview modal */}
      {previewContext && (
        <TextPreviewModal
          filename={previewContext.filename}
          text={previewContext.text}
          onClose={() => setPreviewContext(null)}
        />
      )}
    </div>
  );
}
