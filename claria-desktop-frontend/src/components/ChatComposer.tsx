import type { Ref } from "react";

/**
 * The message composer shared by chat and writer surfaces: a natively
 * resizable textarea plus a send button.
 *
 * Keybinding is uniform across every AI surface: Enter sends, Shift+Enter
 * inserts a newline.
 */
export default function ChatComposer({
  value,
  onChange,
  onSend,
  disabled = false,
  canSend,
  placeholder,
  sendLabel = "Send",
  ariaLabel = "Chat message",
  composerRef,
  rows = 3,
}: {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  /** Disables typing entirely (models loading, pending proposal, busy turn). */
  disabled?: boolean;
  /** Enables the send button; the Enter shortcut respects it too. */
  canSend: boolean;
  placeholder?: string;
  sendLabel?: string;
  ariaLabel?: string;
  composerRef?: Ref<HTMLTextAreaElement>;
  rows?: number;
}) {
  return (
    <div className="flex gap-3">
      <textarea
        ref={composerRef}
        aria-label={ariaLabel}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault();
            if (canSend) onSend();
          }
        }}
        placeholder={placeholder}
        disabled={disabled}
        rows={rows}
        className="flex-1 px-4 py-2.5 text-sm border border-gray-300 rounded-lg resize-y focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50 disabled:text-gray-500"
      />
      <button
        onClick={onSend}
        disabled={!canSend}
        className="self-end px-5 py-2.5 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
      >
        {sendLabel}
      </button>
    </div>
  );
}
