import type { ChatModel } from "../lib/tauri";
import Spinner from "./Spinner";

/**
 * The model dropdown shared by every AI surface, including its loading and
 * failed states. Callers own where the control sits; this owns how the three
 * states look so chat and writer cannot drift.
 */
export default function ModelSelect({
  models,
  loading,
  error,
  value,
  onChange,
  disabled = false,
  ariaLabel = "Chat model",
  className = "",
  defaultOption = false,
}: {
  models: ChatModel[];
  loading: boolean;
  error: string | null;
  value: string;
  onChange: (modelId: string) => void;
  disabled?: boolean;
  ariaLabel?: string;
  className?: string;
  /**
   * Offer an empty-valued row for "no explicit choice". Roles that resolve
   * their own model at call time use it; a picker whose value must be a real
   * model ID leaves it off.
   */
  defaultOption?: boolean;
}) {
  if (loading) {
    return (
      <div className="flex items-center gap-1.5 text-gray-400 text-xs">
        <Spinner />
        <span>Loading models...</span>
      </div>
    );
  }
  if (error) {
    return <span className="text-red-500 text-xs">Failed to load models</span>;
  }
  return (
    <select
      aria-label={ariaLabel}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      disabled={disabled}
      className={`text-xs border border-gray-300 rounded-lg px-2 py-1.5 bg-white focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50 ${className}`}
    >
      {defaultOption && <option value="">Default — chosen automatically</option>}
      {models.length === 0 && !defaultOption && (
        <option value="">No models available</option>
      )}
      {models.map((model) => (
        <option key={model.model_id} value={model.model_id}>
          {model.name}
        </option>
      ))}
    </select>
  );
}
