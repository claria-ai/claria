/**
 * Client-minted identifiers.
 *
 * Backends validate the UUID shape and own the real identity rules — these
 * ids exist so the frontend can name a thing (an idempotency key, an
 * in-flight turn) before the backend has answered about it.
 */
export function randomUuid(): string {
  if (typeof globalThis.crypto?.randomUUID === "function") {
    return globalThis.crypto.randomUUID();
  }
  // Test and webview fallback for contexts where `crypto.randomUUID` is
  // absent (it needs a secure context).
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (value) => {
    const random = Math.floor(Math.random() * 16);
    const nibble = value === "x" ? random : (random & 0x3) | 0x8;
    return nibble.toString(16);
  });
}
