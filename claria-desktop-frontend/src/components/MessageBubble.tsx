import { memo } from "react";
import type { TurnUsage } from "../lib/tauri";
import { MarkdownBlock } from "./Markdown";
import TurnCostBadge from "./TurnCostBadge";

/**
 * One conversation message, shared by chat and writer timelines: user turns
 * are blue and right-aligned, assistant turns render Markdown left-aligned.
 * Memoized — message lists re-render on every composer keystroke.
 */
const MessageBubble = memo(function MessageBubble({
  role,
  content,
  usage,
  writing = false,
}: {
  role: string;
  content: string;
  /**
   * Per-turn usage for the cost badge under assistant turns. `undefined`
   * hides the badge (still loading or surface without cost tracking);
   * `null` marks a legacy turn without recorded usage.
   */
  usage?: TurnUsage | null;
  /**
   * The reply is still arriving. Shows a caret under the text, which is what
   * says "there is more" during the quiet stretch between two paragraphs.
   */
  writing?: boolean;
}) {
  const isUser = role === "user";

  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
      <div
        className={`max-w-[80%] flex flex-col ${isUser ? "items-end" : "items-start"}`}
      >
        <div
          className={`rounded-lg px-4 py-2.5 select-text ${
            isUser ? "bg-blue-600 text-white" : "bg-gray-100 text-gray-800"
          }`}
        >
          {isUser ? (
            <p className="text-sm whitespace-pre-wrap">{content}</p>
          ) : (
            <MarkdownBlock source={content} />
          )}
          {writing && (
            <span
              aria-label="Still writing"
              className="mt-1 inline-block h-3.5 w-1.5 bg-gray-400 motion-safe:animate-pulse"
            />
          )}
        </div>
        {!isUser && <TurnCostBadge usage={usage} />}
      </div>
    </div>
  );
});

export default MessageBubble;
