import { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * The one home for rendered Markdown.
 *
 * Tailwind only generates classes that appear in source as complete literals,
 * so every prose recipe lives here as a full string — never build a prose
 * class with interpolation (`prose-p:${x}` silently generates nothing).
 *
 * Both components are memoized on their source string: chat and writer
 * timelines re-render on every keystroke of their composers, and parsing
 * Markdown for every historical message each time is the single biggest
 * render cost in those views.
 */

export type MarkdownVariant =
  /** Chat bubbles and writer timeline messages: tight spacing. */
  | "chat"
  /** Report canvas body text: document-like paragraph spacing. */
  | "document"
  /** Report canvas bullet items: like document, but no paragraph margins. */
  | "document-compact"
  /** Proposal-card previews: extra-small prose. */
  | "xs"
  /** Plain prose with default spacing (system-prompt modal). */
  | "plain";

const PROSE: Record<MarkdownVariant, string> = {
  chat: "prose prose-sm max-w-none prose-p:my-1 prose-ul:my-1 prose-ol:my-1 prose-li:my-0.5 prose-headings:my-2 prose-pre:my-2 prose-code:text-inherit prose-code:before:content-none prose-code:after:content-none",
  document:
    "prose prose-sm max-w-none prose-headings:my-2 prose-p:my-2 prose-ul:my-2 prose-ol:my-2 prose-li:my-0 prose-code:text-inherit prose-code:before:content-none prose-code:after:content-none",
  "document-compact":
    "prose prose-sm max-w-none prose-headings:my-2 prose-p:my-0 prose-ul:my-2 prose-ol:my-2 prose-li:my-0 prose-code:text-inherit prose-code:before:content-none prose-code:after:content-none",
  xs: "prose prose-xs max-w-none prose-p:my-1",
  plain: "prose prose-sm max-w-none",
};

/** Block-level Markdown inside a prose wrapper. */
export const MarkdownBlock = memo(function MarkdownBlock({
  source,
  variant = "chat",
}: {
  source: string;
  variant?: MarkdownVariant;
}) {
  return (
    <div className={PROSE[variant]}>
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{source}</ReactMarkdown>
    </div>
  );
});

/**
 * Markdown for headings and one-line labels: inline emphasis renders, but the
 * paragraph wrapper is stripped so the text flows inside the parent element.
 */
export const InlineMarkdown = memo(function InlineMarkdown({
  text,
}: {
  text: string;
}) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{ p: ({ children }) => <>{children}</> }}
    >
      {text}
    </ReactMarkdown>
  );
});
