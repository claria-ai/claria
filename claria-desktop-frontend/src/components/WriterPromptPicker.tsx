import type { WriterPrompt } from "../lib/tauri";

/**
 * Dropdown that prefills a writer instruction box from the saved prompt
 * library. Picking is a starting point, not a send: the caller receives the
 * body, puts it in the textarea, and the user edits before submitting.
 *
 * While the library is empty it renders a jump to the Preferences manager
 * instead — the feature would otherwise be invisible until first use.
 */
export default function WriterPromptPicker({
  prompts,
  currentValue,
  disabled,
  onPick,
  onManage,
}: {
  prompts: WriterPrompt[];
  /** The instruction box's current text, guarded before overwriting. */
  currentValue: string;
  disabled: boolean;
  onPick: (body: string) => void;
  /** Open Preferences → Writer Prompts. */
  onManage: () => void;
}) {
  if (prompts.length === 0) {
    return (
      <button
        type="button"
        onClick={onManage}
        disabled={disabled}
        className="text-xs font-medium text-blue-700 hover:text-blue-900 disabled:opacity-50"
      >
        Save reusable prompts…
      </button>
    );
  }

  function pick(promptId: string) {
    const prompt = prompts.find((candidate) => candidate.id === promptId);
    if (!prompt) return;
    if (
      currentValue.trim() !== "" &&
      currentValue.trim() !== prompt.body.trim() &&
      !window.confirm("Replace the current instruction with the saved prompt?")
    ) {
      return;
    }
    onPick(prompt.body);
  }

  return (
    <label className="block">
      <span className="sr-only">Insert saved prompt</span>
      <select
        aria-label="Insert saved prompt"
        value=""
        onChange={(event) => pick(event.target.value)}
        disabled={disabled}
        className="max-w-full rounded-md border border-gray-300 bg-white px-2 py-1 text-xs text-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
      >
        <option value="" disabled>
          Insert saved prompt…
        </option>
        {prompts.map((prompt) => (
          <option key={prompt.id} value={prompt.id}>
            {prompt.name}
          </option>
        ))}
      </select>
    </label>
  );
}
