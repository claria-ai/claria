import { useState, useRef, useEffect, type ReactNode } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  acceptModelAgreement,
  type ChatMessage,
  type ChatModel,
  type TurnUsage,
} from "../lib/tauri";
import {
  EMPTY_SESSION_USAGE,
  accumulateUsage,
  estimateTurnCost,
  formatCost,
  type SessionUsage,
} from "../lib/cost";
import { lookupModelPricing, type ModelPricing } from "../lib/tauri";
import TurnCostBadge from "./TurnCostBadge";
import SessionTotalBanner, { type SpendCaps } from "./SessionTotalBanner";
import LastTurnFooter from "./LastTurnFooter";

function isMarketplaceError(error: string): boolean {
  return error.includes("aws-marketplace:") || error.includes("Marketplace");
}

const DEFAULT_CAPS: SpendCaps = { softUsd: 1.0, hardUsd: 5.0 };

/**
 * `handleSend` contract: components return either a plain string (legacy)
 * or `{ content, usage }`. The widget normalises both.
 */
export type SendResult = string | { content: string; usage: TurnUsage | null };

export default function ChatWidget({
  chatModels,
  chatModelsLoading,
  chatModelsError,
  preferredModelId,
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
  embedded = false,
  caps = DEFAULT_CAPS,
  onOpenPreferences,
  onStartFreshSession,
}: {
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId?: string | null;
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
  embedded?: boolean;
  caps?: SpendCaps;
  /// Optional callback used by the soft- and hard-cap UIs. When omitted,
  /// the buttons are hidden so embedded widgets without a preferences
  /// route don't show dead controls.
  onOpenPreferences?: () => void;
  /// Optional callback for "Start a fresh session" — clears the chat
  /// and resets the session counter. The default just clears the
  /// in-widget messages.
  onStartFreshSession?: () => void;
}) {
  const [messages, setMessages] = useState<ChatMessage[]>(initialMessages ?? []);
  const [usageByIndex, setUsageByIndex] = useState<Array<TurnUsage | null>>(
    initialUsageByIndex ?? []
  );
  const [session, setSession] = useState<SessionUsage>(EMPTY_SESSION_USAGE);
  const [softCapAcknowledged, setSoftCapAcknowledged] = useState(false);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [accepting, setAccepting] = useState(false);

  const hardCapHit = session.totalUsd >= caps.hardUsd && caps.hardUsd > 0;
  const softCapHit =
    session.totalUsd >= caps.softUsd && caps.softUsd > 0 && !hardCapHit;

  // Per-model pricing for pre-flight estimates. Looked up once per
  // selected model and cached.
  const [pricing, setPricing] = useState<ModelPricing | null>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [textareaHeight, setTextareaHeight] = useState(80);
  const dragRef = useRef<{ startY: number; startHeight: number } | null>(null);

  const [selectedModelId, setSelectedModelId] = useState<string | null>(
    initialModelId ?? null
  );

  // Default to preferred model (or first available) once models are loaded
  useEffect(() => {
    if (chatModels.length > 0 && !selectedModelId) {
      const preferred =
        preferredModelId &&
        chatModels.some((m) => m.model_id === preferredModelId)
          ? preferredModelId
          : chatModels[0].model_id;
      setSelectedModelId(preferred);
    }
  }, [chatModels, selectedModelId, preferredModelId]);

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

  // Apply initial messages/model when they change (resume chat)
  useEffect(() => {
    if (initialMessages) setMessages(initialMessages);
    if (initialModelId) setSelectedModelId(initialModelId);
    if (initialUsageByIndex) {
      setUsageByIndex(initialUsageByIndex);
      // Roll up resumed usage so the session banner reflects historical
      // spend on the chat being resumed.
      const resumed = initialUsageByIndex.reduce<SessionUsage>(
        (acc, u) => accumulateUsage(acc, u),
        EMPTY_SESSION_USAGE
      );
      setSession(resumed);
    } else if (initialMessages) {
      // Reset session when initialMessages change without per-index usage.
      setUsageByIndex([]);
      setSession(EMPTY_SESSION_USAGE);
    }
  }, [initialMessages, initialModelId, initialUsageByIndex]);

  // Auto-scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  // Drag-to-resize textarea
  useEffect(() => {
    function onPointerMove(e: PointerEvent) {
      if (!dragRef.current) return;
      const delta = dragRef.current.startY - e.clientY;
      setTextareaHeight(
        Math.max(48, Math.min(400, dragRef.current.startHeight + delta))
      );
    }
    function onPointerUp() {
      dragRef.current = null;
      document.body.style.userSelect = "";
    }
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };
  }, []);

  const canSend =
    !sending &&
    !chatModelsLoading &&
    !extraLoading &&
    !!selectedModelId &&
    !!input.trim() &&
    !hardCapHit;

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
      const result = await onSend(selectedModelId, updatedMessages);
      const content = typeof result === "string" ? result : result.content;
      const usage = typeof result === "string" ? null : result.usage;
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

  function handleStartFresh() {
    setMessages([]);
    setUsageByIndex([]);
    setSession(EMPTY_SESSION_USAGE);
    setSoftCapAcknowledged(false);
    setError(null);
    onStartFreshSession?.();
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
    <div className={`flex flex-col ${embedded ? "flex-1" : "flex-1"}`}>
      {/* Model selector bar */}
      <div
        className={`flex items-center gap-2 px-6 py-2 border-b ${embedded ? "border-gray-100 bg-gray-50" : "border-gray-100 bg-white"}`}
      >
        <div className="flex-1" />
        {chatModelsLoading ? (
          <div className="flex items-center gap-1.5 text-gray-400 text-xs">
            <Spinner />
            <span>Loading models...</span>
          </div>
        ) : chatModelsError ? (
          <span className="text-red-500 text-xs">Failed to load models</span>
        ) : (
          <select
            value={selectedModelId ?? ""}
            onChange={(e) => setSelectedModelId(e.target.value)}
            className="text-xs border border-gray-300 rounded-lg px-2 py-1.5 bg-white focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
          >
            {chatModels.map((m) => (
              <option key={m.model_id} value={m.model_id}>
                {m.name}
              </option>
            ))}
          </select>
        )}
      </div>

      {/* Persistent session-cost banner — fuel-gauge style. */}
      <SessionTotalBanner session={session} caps={caps} />

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
          <div className="text-center text-gray-400 text-sm mt-8">
            <p className="mb-1">{emptyStateTitle}</p>
            {emptyStateSubtitle && (
              <p className="text-xs">{emptyStateSubtitle}</p>
            )}
          </div>
        )}

        {messages.map((msg, i) => (
          <MessageBubble key={i} message={msg} usage={usageByIndex[i]} />
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
        sessionTotalUsd={session.totalUsd}
      />

      {/* Soft-cap warning — advisory, dismissable for the session. */}
      {softCapHit && !softCapAcknowledged && (
        <SoftCapBanner
          sessionUsd={session.totalUsd}
          softCapUsd={caps.softUsd}
          hardCapUsd={caps.hardUsd}
          onContinue={() => setSoftCapAcknowledged(true)}
          onAdjust={onOpenPreferences}
        />
      )}

      {/* Input bar — replaced by hard-cap lockout when over the limit. */}
      {hardCapHit ? (
        <HardCapLockout
          sessionUsd={session.totalUsd}
          hardCapUsd={caps.hardUsd}
          turnCount={session.turnCount}
          onAdjust={onOpenPreferences}
          onStartFresh={handleStartFresh}
        />
      ) : (
        <div className="border-t border-gray-200 bg-white">
          {/* Drag handle */}
          <div
            className="flex justify-center py-1.5 cursor-row-resize select-none hover:bg-gray-50 transition-colors"
            onPointerDown={(e) => {
              dragRef.current = {
                startY: e.clientY,
                startHeight: textareaHeight,
              };
              document.body.style.userSelect = "none";
            }}
          >
            <div className="w-8 h-1 rounded-full bg-gray-300" />
          </div>
          <div className="flex gap-3 px-6 pb-4">
            <textarea
              ref={textareaRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  handleSend();
                }
              }}
              placeholder={resolvedPlaceholder}
              disabled={
                sending ||
                chatModelsLoading ||
                extraLoading ||
                !selectedModelId
              }
              style={{ height: textareaHeight, resize: "none" }}
              className="flex-1 px-4 py-2.5 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50"
            />
            <button
              onClick={handleSend}
              disabled={!canSend}
              className="self-end px-5 py-2.5 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
            >
              Send
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

import { formatCost } from "../lib/cost";

function SoftCapBanner({
  sessionUsd,
  softCapUsd,
  hardCapUsd,
  onContinue,
  onAdjust,
}: {
  sessionUsd: number;
  softCapUsd: number;
  hardCapUsd: number;
  onContinue: () => void;
  onAdjust?: () => void;
}) {
  return (
    <div className="border-t border-amber-200 bg-amber-50 px-6 py-3 text-xs text-amber-900 flex flex-wrap items-center gap-3">
      <span>
        You&apos;ve spent {formatCost(sessionUsd)} in this chat session. That&apos;s past
        your soft warning of {formatCost(softCapUsd)}. You can keep going — Claria
        will block sending at {formatCost(hardCapUsd)}.
      </span>
      <div className="flex gap-2 ml-auto">
        <button
          onClick={onContinue}
          className="px-3 py-1 text-xs rounded bg-amber-600 text-white hover:bg-amber-700"
        >
          Continue
        </button>
        {onAdjust && (
          <button
            onClick={onAdjust}
            className="px-3 py-1 text-xs rounded border border-amber-300 text-amber-800 hover:bg-amber-100"
          >
            Adjust limits
          </button>
        )}
      </div>
    </div>
  );
}

function HardCapLockout({
  sessionUsd,
  hardCapUsd,
  turnCount,
  onAdjust,
  onStartFresh,
}: {
  sessionUsd: number;
  hardCapUsd: number;
  turnCount: number;
  onAdjust?: () => void;
  onStartFresh: () => void;
}) {
  return (
    <div className="border-t border-red-200 bg-red-50 px-6 py-4 text-sm text-red-900 flex flex-col gap-2">
      <p className="font-medium">Session cap of {formatCost(hardCapUsd)} reached.</p>
      <p className="text-xs">
        To protect against runaway charges, Claria has paused this session. You&apos;ve
        sent {turnCount} message{turnCount === 1 ? "" : "s"} costing{" "}
        {formatCost(sessionUsd)} so far.
      </p>
      <div className="flex gap-2 mt-1">
        {onAdjust && (
          <button
            onClick={onAdjust}
            className="px-3 py-1.5 text-xs rounded bg-red-600 text-white hover:bg-red-700"
          >
            Raise the cap in Preferences
          </button>
        )}
        <button
          onClick={onStartFresh}
          className="px-3 py-1.5 text-xs rounded border border-red-300 text-red-800 hover:bg-red-100"
        >
          Start a fresh session
        </button>
      </div>
    </div>
  );
}

function MessageBubble({
  message,
  usage,
}: {
  message: ChatMessage;
  usage?: TurnUsage | null;
}) {
  const isUser = message.role === "user";

  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
      <div className={`max-w-[80%] flex flex-col ${isUser ? "items-end" : "items-start"}`}>
        <div
          className={`rounded-lg px-4 py-2.5 ${
            isUser ? "bg-blue-600 text-white" : "bg-gray-100 text-gray-800"
          }`}
        >
          {isUser ? (
            <p className="text-sm whitespace-pre-wrap">{message.content}</p>
          ) : (
            <div className="prose prose-sm max-w-none prose-p:my-1 prose-ul:my-1 prose-ol:my-1 prose-li:my-0.5 prose-headings:my-2 prose-pre:my-2 prose-code:text-inherit prose-code:before:content-none prose-code:after:content-none">
              <Markdown remarkPlugins={[remarkGfm]}>{message.content}</Markdown>
            </div>
          )}
        </div>
        {!isUser && <TurnCostBadge usage={usage} />}
      </div>
    </div>
  );
}

function Spinner() {
  return (
    <svg className="animate-spin h-3.5 w-3.5" viewBox="0 0 24 24" fill="none">
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
  );
}
