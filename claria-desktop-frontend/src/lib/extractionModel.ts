import type { ChatModel } from "./tauri";

/** An unsuccessful catalog lookup is not evidence that a saved model vanished. */
export function extractionModelUnavailable(id: string | null, models: ChatModel[], loading: boolean, error: string | null): boolean {
  return !!id && !loading && error === null && !models.some((model) => model.model_id === id);
}
