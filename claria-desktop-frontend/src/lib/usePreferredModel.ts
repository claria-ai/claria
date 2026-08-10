import { useCallback, useMemo, useState } from "react";
import type { ChatModel } from "./tauri";

/**
 * Shared model-selection defaulting for every AI surface.
 *
 * The user's explicit choice is kept while it exists in the loaded model
 * list; otherwise the selection falls back to the preferred model, then the
 * first available model, then `""` (nothing selectable yet). The fallback is
 * derived, so a late-loading model list never needs a reconciliation effect.
 */
export function usePreferredModel(
  models: ChatModel[],
  preferredModelId?: string | null,
  initialModelId?: string | null
) {
  const [chosenModelId, setChosenModelId] = useState(initialModelId ?? "");

  const selectedModelId = useMemo(() => {
    if (
      chosenModelId &&
      models.some((model) => model.model_id === chosenModelId)
    ) {
      return chosenModelId;
    }
    const preferred = models.find(
      (model) => model.model_id === preferredModelId
    );
    return preferred?.model_id ?? models[0]?.model_id ?? "";
  }, [chosenModelId, models, preferredModelId]);

  const setSelectedModelId = useCallback((modelId: string) => {
    setChosenModelId(modelId);
  }, []);

  return [selectedModelId, setSelectedModelId] as const;
}
