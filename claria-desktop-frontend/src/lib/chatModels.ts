import { createContext, useContext } from "react";
import type { ChatModel } from "./tauri";

/**
 * App-wide chat-model availability: the Bedrock model list (loaded once at
 * startup by App), its loading/error state, the user's preferred model, and
 * a retry hook. Every AI surface reads this through `useChatModels()`
 * instead of prop-drilling five values through the page tree.
 */
export type ChatModelsState = {
  models: ChatModel[];
  loading: boolean;
  error: string | null;
  preferredModelId: string | null;
  /** Re-fetch the model list after a failure. */
  retry: () => void;
  /** Record a new preferred model (persisted by the caller). */
  setPreferredModelId: (id: string | null) => void;
};

/** Provided by App; the default covers detached test mounts. */
export const ChatModelsContext = createContext<ChatModelsState>({
  models: [],
  loading: true,
  error: null,
  preferredModelId: null,
  retry: () => {},
  setPreferredModelId: () => {},
});

export function useChatModels(): ChatModelsState {
  return useContext(ChatModelsContext);
}
