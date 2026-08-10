import { useState, useRef, useEffect, type ReactNode } from "react";
import {
  acceptModelAgreement,
  type ChatMessage,
  type TurnUsage,
} from "../lib/tauri";
import { useChatModels } from "../lib/chatModels";
import {
  EMPTY_SESSION_USAGE,
  accumulateUsage,
  estimateTurnCost,
  formatCost,
  type SessionUsage,
} from "../lib/cost";
import { lookupModelPricing, type ModelPricing } from "../lib/tauri";
import { usePreferredModel } from "../lib/usePreferredModel";
import ChatComposer from "./ChatComposer";
import ChatEmptyState from "./ChatEmptyState";
import MessageBubble from "./MessageBubble";
import ModelSelect from "./ModelSelect";
import SessionTotalBanner from "./SessionTotalBanner";
import LastTurnFooter from "./LastTurnFooter";
import Spinner from "./Spinner";

function isMarketplaceError(error: string): boolean {
  return error.includes("aws-marketplace:") || error.includes("Marketplace");
}

/** `handleSend` contract: the assistant reply plus its token usage, if known. */
export type SendResult = { content: string; usage: TurnUsage | null };

export default function ChatWidget({
  onSend,
  initialMessages,
  initialModelId,
  initialUsageByIndex,
  contextTokens,
  emptyStateTitle = "Start the conversation.",
  emptyStateSubtitle,
  placeholder,
  extraLoading = false,
  extraLoadingText = "Loading...",
  toolbar,
  historyHeader,
}: {
  onSend: (modelId: string, messages: ChatMessage[]) => Promise<SendResult>;
  initialMessages?: ChatMessage[];
  initialModelId?: string;
  /// Per-message usage aligned with `initialMessages` — used when
  /// resuming a chat from history so old assistant turns can render
  /// their cost badges.
  initialUsageByIndex?: Array<TurnUsage | null>;
  /// Optional context-token count used for pre-flight cost estimation.
  /// When omitted, the pre-flight chip is hidden.
  contextTokens?: number | null;
  emptyStateTitle?: string;
  emptyStateSubtitle?: string;
  placeholder?: string;
  extraLoading?: boolean;
  extraLoadingText?: string;
  toolbar?: ReactNode;
  /// Optional content rendered between the session banner and the
  /// toolbar — typically a chat-history summary header on resume.
  historyHeader?: ReactNode;
}) {
  const [messages, setMessages] = useState<ChatMessage[]>(initialMessages ?? []);
  const [usageByIndex, setUsageByIndex] = useState<Array<TurnUsage | null>>(
    initialUsageByIndex ?? []
  );
  // Roll up resumed usage so the session banner reflects historical spend
  // on the chat being resumed. Initial props are plain initial state — a
  // different chat identity remounts the widget via the caller's `key`.
  const [session, setSession] = useState<SessionUsage>(() =>
    (initialUsageByIndex ?? []).reduce<SessionUsage>(
      (acc, usage) => accumulateUsage(acc, usage),
      EMPTY_SESSION_USAGE
    )
  );
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [accepting, setAccepting] = useState(false);

  // Per-model pricing for pre-flight estimates. Looked up once per
  // selected model and cached.
  const [pricing, setPricing] = useState<ModelPricing | null>(null);

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

  // Resolve pricing for the selected model. `null` when unknown.
  useEffect(() => {
    if (!selectedModelId) {
      setPricing(null);
      return;
    }
    let cancelled = false;
    lookupModelPricing(selectedModelId)
      .then((p) => {
        if (!cancelled) setPricing(p);
      })
      .catch(() => {
        if (!cancelled) setPricing(null);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedModelId]);

  // Auto-scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

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
    // Pad usageByIndex for the new user message so indices align.
    setUsageByIndex((prev) => [...prev, null]);

    setSending(true);
    try {
      const { content, usage } = await onSend(selectedModelId, updatedMessages);
      const assistantMessage: ChatMessage = {
        role: "assistant",
        content,
      };
      setMessages([...updatedMessages, assistantMessage]);
      setUsageByIndex((prev) => [...prev, usage]);
      setSession((prev) => accumulateUsage(prev, usage));
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
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
    <div className="flex flex-col flex-1">
      {/* Model selector bar */}
      <div className="flex items-center gap-2 px-6 py-2 border-b border-gray-100 bg-white">
        <div className="flex-1" />
        <ModelSelect
          models={chatModels}
          loading={chatModelsLoading}
          error={chatModelsError}
          value={selectedModelId}
          onChange={setSelectedModelId}
        />
      </div>

      {/* Persistent session-cost banner — fuel-gauge style. */}
      <SessionTotalBanner session={session} />

      {/* Optional resumed-chat history header. */}
      {historyHeader}

      {/* Optional toolbar slot (context pills, etc.) */}
      {toolbar}

      {/* Extra loading indicator */}
      {extraLoading && (
        <div className="flex items-center gap-2 px-6 py-2 border-b border-gray-100 bg-white">
          <Spinner />
          <span className="text-xs text-gray-400">{extraLoadingText}</span>
        </div>
      )}

      {/* Messages area */}
      <div className="flex-1 overflow-y-auto px-6 py-4 space-y-4">
        {messages.length === 0 && !sending && (
          <ChatEmptyState title={emptyStateTitle} subtitle={emptyStateSubtitle} />
        )}

        {messages.map((msg, i) => (
          <MessageBubble
            key={i}
            role={msg.role}
            content={msg.content}
            usage={usageByIndex[i]}
          />
        ))}

        {sending && (
          <div className="flex items-start gap-3">
            <div className="bg-gray-100 rounded-lg px-4 py-2.5 max-w-[80%]">
              <div className="flex items-center gap-2 text-gray-500 text-sm">
                <Spinner />
                <span>Thinking...</span>
                {(() => {
                  const est = estimateTurnCost(
                    pricing,
                    contextTokens ?? 0,
                    input.length
                  );
                  return est != null && est > 0 ? (
                    <span className="text-[10px] text-gray-400">
                      (~{formatCost(est)} for this turn)
                    </span>
                  ) : null;
                })()}
              </div>
            </div>
          </div>
        )}

        {error && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-3">
            <p className="text-red-800 text-sm">{error}</p>
            {isMarketplaceError(error) && selectedModelId && (
              <button
                onClick={handleAcceptAgreement}
                disabled={accepting}
                className="mt-2 px-4 py-1.5 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
              >
                {accepting ? "Accepting..." : "Accept Model Agreement"}
              </button>
            )}
          </div>
        )}

        <div ref={messagesEndRef} />
      </div>

      {/* Last-turn footer — quiet ambient line above the composer. */}
      <LastTurnFooter
        usage={
          messages.length > 0 && messages[messages.length - 1].role === "assistant"
            ? usageByIndex[messages.length - 1]
            : undefined
        }
        sessionTotalUsd={session.unknownCostTurns === 0 ? session.totalUsd : null}
      />

      {/* Input bar */}
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
  );
}
