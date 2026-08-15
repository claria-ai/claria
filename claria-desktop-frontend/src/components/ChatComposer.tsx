import type { Ref } from "react";

/**
 * The message composer shared by chat and writer surfaces: a natively
 * resizable textarea plus a send button, and — where the surface can end a
 * turn early — a stop button beside it.
 *
 * Keybinding is uniform across every AI surface: Enter sends, Shift+Enter
 * inserts a newline.
 */
export default function ChatComposer({
  value,
  onChange,
  onSend,
  onStop,
  canStop = false,
  disabled = false,
  canSend,
  placeholder,
  sendLabel = "Send",
  stopLabel = "Stop",
  ariaLabel = "Chat message",
  composerRef,
  rows = 3,
}: {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  /**
   * Ends the turn in flight. Passing it shows the stop button at all times
   * so the control doesn't appear and disappear under the cursor; it is
   * enabled only while `canStop`.
   */
  onStop?: () => void;
  canStop?: boolean;
  /** Disables typing entirely (models loading, pending proposal, busy turn). */
  disabled?: boolean;
  /** Enables the send button; the Enter shortcut respects it too. */
  canSend: boolean;
  placeholder?: string;
  sendLabel?: string;
  stopLabel?: string;
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
      <div className="self-end flex flex-col gap-2">
        <button
          onClick={onSend}
          disabled={!canSend}
          className="px-5 py-2.5 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
        >
          {sendLabel}
        </button>
        {onStop && (
          <button
            onClick={onStop}
            disabled={!canStop}
            title="Stop the reply and keep what has arrived"
            className="px-5 py-1.5 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-lg hover:bg-gray-100 transition-colors disabled:opacity-40 disabled:hover:bg-white"
          >
            {stopLabel}
          </button>
        )}
      </div>
    </div>
  );
}
