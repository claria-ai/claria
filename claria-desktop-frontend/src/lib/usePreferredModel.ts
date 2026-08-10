import { useEffect, useState } from "react";
import type { ChatModel } from "./tauri";

/**
 * Shared model-selection defaulting for every AI surface.
 *
 * Keeps the current selection while it exists in the loaded model list;
 * otherwise falls back to the user's preferred model, then the first
 * available model, then `""` (nothing selectable yet).
 */
export function usePreferredModel(
  models: ChatModel[],
  preferredModelId?: string | null,
  initialModelId?: string | null
) {
  const [selectedModelId, setSelectedModelId] = useState(initialModelId ?? "");

  useEffect(() => {
    if (
      selectedModelId &&
      models.some((model) => model.model_id === selectedModelId)
    ) {
      return;
    }
    const preferred = models.find(
      (model) => model.model_id === preferredModelId
    );
    setSelectedModelId(preferred?.model_id ?? models[0]?.model_id ?? "");
  }, [models, preferredModelId, selectedModelId]);

  return [selectedModelId, setSelectedModelId] as const;
}
