import { useState, useRef, useEffect, useMemo, type ReactNode } from "react";
import {
  acceptModelAgreement,
  type ChatMessage,
  type TurnUsage,
} from "../lib/tauri";
import { useChatModels } from "../lib/chatModels";
import {
  EMPTY_SESSION_USAGE,
  accumulateUsage,
  type SessionUsage,
} from "../lib/cost";
import { buildCostLedger } from "../lib/costLedger";
import { usePreferredModel } from "../lib/usePreferredModel";
import { usePricingMap } from "../lib/usePricingMap";
import ChatComposer from "./ChatComposer";
import ChatEmptyState from "./ChatEmptyState";
import MessageBubble from "./MessageBubble";
import ModelSelect from "./ModelSelect";
import SessionTabs, { UsageTabIcon } from "./SessionTabs";
import SessionUsagePanel from "./SessionUsagePanel";
import Spinner from "./Spinner";

function isMarketplaceError(error: string): boolean {
  return error.includes("aws-marketplace:") || error.includes("Marketplace");
}

/** `handleSend` contract: the assistant reply plus its token usage, if known. */
export type SendResult = { content: string; usage: TurnUsage | null };

/**
 * `handleSend` also receives an `onDelta` callback. Senders whose backend
 * streams call it with each incremental text chunk; senders that don't
 * simply ignore it, and the widget falls back to the awaited result.
 */
export type SendFn = (
  modelId: string,
  messages: ChatMessage[],
  onDelta: (text: string) => void
) => Promise<SendResult>;

export default function ChatWidget({
  onSend,
  initialMessages,
  initialModelId,
  initialUsageByIndex,
  initialTimestampsByIndex,
  emptyStateTitle = "Start the conversation.",
  emptyStateSubtitle,
  placeholder,
  extraLoading = false,
  extraLoadingText = "Loading...",
  toolbar,
  historyHeader,
  usageActions,
}: {
  onSend: SendFn;
  initialMessages?: ChatMessage[];
  initialModelId?: string;
  /// Per-message usage aligned with `initialMessages` — used when
  /// resuming a chat from history so old assistant turns can appear in the
  /// usage tab and render badges when the user enables them.
  initialUsageByIndex?: Array<TurnUsage | null>;
  initialTimestampsByIndex?: Array<string | null>;
  emptyStateTitle?: string;
  emptyStateSubtitle?: string;
  placeholder?: string;
  extraLoading?: boolean;
  extraLoadingText?: string;
  toolbar?: ReactNode;
  /// Optional quiet resume context rendered above the conversation toolbar.
  historyHeader?: ReactNode;
  /** Optional actions that belong with costs rather than in the chat flow. */
  usageActions?: ReactNode;
}) {
  const [messages, setMessages] = useState<ChatMessage[]>(initialMessages ?? []);
  const [usageByIndex, setUsageByIndex] = useState<Array<TurnUsage | null>>(
    initialUsageByIndex ?? []
  );
  const [timestampsByIndex, setTimestampsByIndex] = useState<
    Array<string | null>
  >(initialTimestampsByIndex ?? []);
  // Roll up resumed usage for the dedicated session-usage tab. Initial props
  // are plain initial state — a
  // different chat identity remounts the widget via the caller's `key`.
  const [session, setSession] = useState<SessionUsage>(() =>
    (initialUsageByIndex ?? []).reduce<SessionUsage>(
      (acc, usage) => accumulateUsage(acc, usage),
      EMPTY_SESSION_USAGE
    )
  );
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  // Incrementally streamed assistant text for the in-flight turn. `null`
  // until the first delta arrives — non-streaming senders (and the mock
  // paths in tests) never set it, and the widget falls back to the awaited
  // response.
  const [streamText, setStreamText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [accepting, setAccepting] = useState(false);
  const [activeTab, setActiveTab] = useState<"conversation" | "usage">(
    "conversation"
  );
  const [showTurnCosts, setShowTurnCosts] = useState(false);

  const messagesEndRef = useRef<HTMLDivElement>(null);

  const {
    models: chatModels,
    loading: chatModelsLoading,
    error: chatModelsError,
    preferredModelId,
  } = useChatModels();
  const [selectedModelId, setSelectedModelId] = usePreferredModel(
    chatModels,
    preferredModelId,
    initialModelId
  );

  // Ordered assistant-turn usages for the cache-aware ledger. Only
  // assistant turns are metered; a null slot is a legacy/unmetered turn.
  const assistantTurnUsages = useMemo(
    () =>
      messages
        .map((message, index) => ({ message, index }))
        .filter(({ message }) => message.role === "assistant")
        .map(({ index }) => usageByIndex[index] ?? null),
    [messages, usageByIndex]
  );
  const assistantTurnTimestamps = useMemo(
    () =>
      messages
        .map((message, index) => ({ message, index }))
        .filter(({ message }) => message.role === "assistant")
        .map(({ index }) => timestampsByIndex[index] ?? null),
    [messages, timestampsByIndex]
  );
  const turnModelIds = useMemo(
    () => assistantTurnUsages.flatMap((usage) => (usage ? [usage.model_id] : [])),
    [assistantTurnUsages]
  );
  const pricingByModel = usePricingMap(
    activeTab === "usage" ? turnModelIds : []
  );
  const ledger = useMemo(
    () =>
      buildCostLedger(
        assistantTurnUsages,
        pricingByModel,
        assistantTurnTimestamps
      ),
    [assistantTurnTimestamps, assistantTurnUsages, pricingByModel]
  );

  // Auto-scroll to bottom when messages change or streamed text grows.
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, streamText]);

  const canSend =
    !sending &&
    !chatModelsLoading &&
    !extraLoading &&
    !!selectedModelId &&
    !!input.trim();

  async function handleSend() {
    const text = input.trim();
    if (!text || sending || !selectedModelId) return;

    setInput("");
    setError(null);

    const userMessage: ChatMessage = { role: "user", content: text };
    const updatedMessages = [...messages, userMessage];
    setMessages(updatedMessages);
    // Pad metadata for the new user message so indices align.
    setUsageByIndex((prev) => [...prev, null]);
    setTimestampsByIndex((prev) => [...prev, new Date().toISOString()]);

    setSending(true);
    try {
      const { content, usage } = await onSend(
        selectedModelId,
        updatedMessages,
        (delta) => setStreamText((prev) => (prev ?? "") + delta)
      );
      const assistantMessage: ChatMessage = {
        role: "assistant",
        content,
      };
      setMessages([...updatedMessages, assistantMessage]);
      setUsageByIndex((prev) => [...prev, usage]);
      setTimestampsByIndex((prev) => [...prev, new Date().toISOString()]);
      setSession((prev) => accumulateUsage(prev, usage));
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
      setStreamText(null);
    }
  }

  async function handleAcceptAgreement() {
    if (!selectedModelId || accepting) return;
    const bareModelId = selectedModelId.replace(/^[a-z]+\./, "");
    setAccepting(true);
    try {
      await acceptModelAgreement(bareModelId);
      setError(null);
    } catch (e) {
      setError(`Failed to accept agreement: ${String(e)}`);
    } finally {
      setAccepting(false);
    }
  }

  const resolvedPlaceholder =
    placeholder ??
    (extraLoading
      ? extraLoadingText
      : chatModelsLoading
        ? "Loading models..."
        : "Type a message...");

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-gray-100 bg-white px-6 py-2">
        <div className="flex-1" />
        <ModelSelect
          models={chatModels}
          loading={chatModelsLoading}
          error={chatModelsError}
          value={selectedModelId}
          onChange={setSelectedModelId}
        />
      </div>

      <SessionTabs
        idPrefix="chat-session"
        label="Chat session"
        active={activeTab}
        onSelect={setActiveTab}
        tabs={[
          { id: "conversation", label: "Conversation" },
          {
            id: "usage",
            label: "Costs and cache",
            compact: true,
            icon: <UsageTabIcon />,
          },
        ]}
      />

      {activeTab === "usage" ? (
        <div
          id="chat-session-panel-usage"
          role="tabpanel"
          aria-labelledby="chat-session-tab-usage"
          className="min-h-0 flex-1"
        >
          <SessionUsagePanel
            session={session}
            ledger={ledger}
            showTurnCosts={showTurnCosts}
            onShowTurnCostsChange={setShowTurnCosts}
            actions={usageActions}
          />
        </div>
      ) : (
        <div
          id="chat-session-panel-conversation"
          role="tabpanel"
          aria-labelledby="chat-session-tab-conversation"
          className="flex min-h-0 flex-1 flex-col"
        >
          {historyHeader}
          {toolbar}

          {extraLoading && (
            <div className="flex items-center gap-2 border-b border-gray-100 bg-white px-6 py-2">
              <Spinner />
              <span className="text-xs text-gray-400">{extraLoadingText}</span>
            </div>
          )}

          <div className="flex-1 space-y-4 overflow-y-auto px-6 py-4">
            {messages.length === 0 && !sending && (
              <ChatEmptyState
                title={emptyStateTitle}
                subtitle={emptyStateSubtitle}
              />
            )}

            {messages.map((msg, i) => (
              <MessageBubble
                key={i}
                role={msg.role}
                content={msg.content}
                usage={showTurnCosts ? usageByIndex[i] : undefined}
              />
            ))}

            {sending && streamText !== null && (
              <MessageBubble role="assistant" content={streamText} />
            )}

            {sending && streamText === null && (
              <div className="flex items-start gap-3">
                <div className="max-w-[80%] rounded-lg bg-gray-100 px-4 py-2.5">
                  <div className="flex items-center gap-2 text-sm text-gray-500">
                    <Spinner />
                    <span>Thinking...</span>
                  </div>
                </div>
              </div>
            )}

            {error && (
              <div className="rounded-lg border border-red-200 bg-red-50 p-3">
                <p className="text-sm text-red-800">{error}</p>
                {isMarketplaceError(error) && selectedModelId && (
                  <button
                    onClick={handleAcceptAgreement}
                    disabled={accepting}
                    className="mt-2 rounded-lg bg-blue-600 px-4 py-1.5 text-sm text-white transition-colors hover:bg-blue-700 disabled:opacity-50"
                  >
                    {accepting ? "Accepting..." : "Accept Model Agreement"}
                  </button>
                )}
              </div>
            )}

            <div ref={messagesEndRef} />
          </div>

          <div className="border-t border-gray-200 bg-white px-6 py-4">
            <ChatComposer
              value={input}
              onChange={setInput}
              onSend={() => void handleSend()}
              disabled={
                sending || chatModelsLoading || extraLoading || !selectedModelId
              }
              canSend={canSend}
              placeholder={resolvedPlaceholder}
            />
          </div>
        </div>
      )}
    </div>
  );
}
