import { useState, useRef, useEffect, useCallback } from "react";
import Markdown from "react-markdown";
import {
  acceptModelAgreement,
  chatMessage,
  listChatModels,
  type ChatMessage,
  type ChatModel,
} from "../lib/tauri";
import type { Page } from "../App";

function isMarketplaceError(error: string): boolean {
  return error.includes("aws-marketplace:") || error.includes("Marketplace");
}

export default function ClientChat({
  navigate,
  clientId: _clientId,
  clientName,
}: {
  navigate: (page: Page) => void;
  clientId: string;
  clientName: string;
}) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [accepting, setAccepting] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Model state
  const [models, setModels] = useState<ChatModel[]>([]);
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);
  const [modelsLoading, setModelsLoading] = useState(true);
  const [modelsError, setModelsError] = useState<string | null>(null);

  const loadModels = useCallback(async () => {
    setModelsLoading(true);
    setModelsError(null);
    try {
      const result = await listChatModels();
      setModels(result);
      if (result.length > 0 && !selectedModelId) {
        setSelectedModelId(result[0].model_id);
      }
    } catch (e) {
      setModelsError(String(e));
    } finally {
      setModelsLoading(false);
    }
  }, [selectedModelId]);

  useEffect(() => {
    loadModels();
  }, [loadModels]);

  // Auto-scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const canSend = !sending && !modelsLoading && !!selectedModelId && !!input.trim();

  async function handleSend() {
    const text = input.trim();
    if (!text || sending || !selectedModelId) return;

    setInput("");
    setError(null);

    const userMessage: ChatMessage = { role: "user", content: text };
    const updatedMessages = [...messages, userMessage];
    setMessages(updatedMessages);

    setSending(true);
    try {
      const response = await chatMessage(selectedModelId, updatedMessages);
      const assistantMessage: ChatMessage = {
        role: "assistant",
        content: response,
      };
      setMessages([...updatedMessages, assistantMessage]);
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
    }
  }

  async function handleAcceptAgreement() {
    if (!selectedModelId || accepting) return;

    // The inference profile ID is like "us.anthropic.claude-sonnet-4-20250514-v1:0"
    // but the agreement API needs the bare model ID like "anthropic.claude-sonnet-4-20250514-v1:0"
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

  return (
    <div className="flex flex-col h-screen">
      {/* Header */}
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
          <p className="text-xs text-gray-400">Client intake chat</p>
        </div>

        {/* Model selector */}
        <div className="flex items-center gap-2">
          {modelsLoading ? (
            <div className="flex items-center gap-1.5 text-gray-400 text-xs">
              <Spinner />
              <span>Loading models...</span>
            </div>
          ) : modelsError ? (
            <span className="text-red-500 text-xs">Failed to load models</span>
          ) : (
            <select
              value={selectedModelId ?? ""}
              onChange={(e) => setSelectedModelId(e.target.value)}
              className="text-xs border border-gray-300 rounded-lg px-2 py-1.5 bg-white focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            >
              {models.map((m) => (
                <option key={m.model_id} value={m.model_id}>
                  {m.name}
                </option>
              ))}
            </select>
          )}
        </div>
      </div>

      {/* Messages area */}
      <div className="flex-1 overflow-y-auto px-6 py-4 space-y-4">
        {messages.length === 0 && !sending && (
          <div className="text-center text-gray-400 text-sm mt-8">
            <p className="mb-1">Start the conversation.</p>
            <p className="text-xs">
              The assistant will help you gather intake information for this
              client.
            </p>
          </div>
        )}

        {messages.map((msg, i) => (
          <MessageBubble key={i} message={msg} />
        ))}

        {sending && (
          <div className="flex items-start gap-3">
            <div className="bg-gray-100 rounded-lg px-4 py-2.5 max-w-[80%]">
              <div className="flex items-center gap-2 text-gray-500 text-sm">
                <Spinner />
                <span>Thinking...</span>
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

      {/* Input bar */}
      <div className="border-t border-gray-200 bg-white px-6 py-4">
        <div className="flex gap-3">
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && !e.shiftKey && handleSend()}
            placeholder={modelsLoading ? "Loading models..." : "Type a message..."}
            disabled={sending || modelsLoading || !selectedModelId}
            className="flex-1 px-4 py-2.5 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50"
          />
          <button
            onClick={handleSend}
            disabled={!canSend}
            className="px-5 py-2.5 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
          >
            Send
          </button>
        </div>
      </div>
    </div>
  );
}

function MessageBubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === "user";

  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
      <div
        className={`rounded-lg px-4 py-2.5 max-w-[80%] ${
          isUser
            ? "bg-blue-600 text-white"
            : "bg-gray-100 text-gray-800"
        }`}
      >
        {isUser ? (
          <p className="text-sm whitespace-pre-wrap">{message.content}</p>
        ) : (
          <div className="prose prose-sm max-w-none prose-p:my-1 prose-ul:my-1 prose-ol:my-1 prose-li:my-0.5 prose-headings:my-2 prose-pre:my-2 prose-code:text-inherit prose-code:before:content-none prose-code:after:content-none">
            <Markdown>{message.content}</Markdown>
          </div>
        )}
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
