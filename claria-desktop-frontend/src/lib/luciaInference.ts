import { logFrontendEvent } from "./logBridge";

type Listener = () => void;

const pendingByClient = new Map<string, Set<symbol>>();
const listenersByClient = new Map<string, Set<Listener>>();

function notify(clientId: string): void {
  listenersByClient.get(clientId)?.forEach((listener) => {
    try {
      listener();
    } catch {
      // A cosmetic observer cannot prevent a command from starting or change
      // its settled result. The diagnostic contains no client or request data.
      logFrontendEvent("error", "Lucia inference listener failed");
    }
  });
}

/** Pending inference-bearing commands, including work whose view has unmounted. */
export function getLuciaInferenceCount(clientId: string): number {
  return pendingByClient.get(clientId)?.size ?? 0;
}

/** Subscribe to one client's count; read the current value through the snapshot. */
export function subscribeLuciaInference(
  clientId: string,
  listener: Listener,
): () => void {
  let listeners = listenersByClient.get(clientId);
  if (!listeners) {
    listeners = new Set();
    listenersByClient.set(clientId, listeners);
  }
  // Each subscription owns its entry, even when callers reuse a callback.
  const subscription = () => listener();
  listeners.add(subscription);
  return () => {
    listeners.delete(subscription);
    if (listeners.size === 0 && listenersByClient.get(clientId) === listeners) {
      listenersByClient.delete(clientId);
    }
  };
}

/**
 * Follow a real command's lifetime without changing its result or cancellation.
 * Removing a view's subscription does not end its requests: only settlement
 * retires a token, so overlapping requests cannot clear each other's activity.
 */
export async function withLuciaInference<T>(
  clientId: string,
  operation: () => Promise<T>,
): Promise<T> {
  let pending = pendingByClient.get(clientId);
  if (!pending) {
    pending = new Set();
    pendingByClient.set(clientId, pending);
  }
  const token = Symbol();
  pending.add(token);
  try {
    notify(clientId);
    return await operation();
  } finally {
    pending.delete(token);
    if (pending.size === 0) pendingByClient.delete(clientId);
    notify(clientId);
  }
}
