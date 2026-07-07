import { useState } from "react";

// Shared "more mode" lifecycle: clock-toggle + lazily fetched deleted items.
// Toggling off keeps the cached list; every re-enable refetches.
export function useMoreMode<T>(
  fetchItems: () => Promise<T[]>,
  onError: (e: unknown) => void,
) {
  const [moreMode, setMoreMode] = useState(false);
  const [deletedItems, setDeletedItems] = useState<T[]>([]);
  const [deletedLoading, setDeletedLoading] = useState(false);
  const [restoringKey, setRestoringKey] = useState<string | null>(null);

  async function toggleMoreMode() {
    const next = !moreMode;
    setMoreMode(next);
    if (next) {
      setDeletedLoading(true);
      try {
        setDeletedItems(await fetchItems());
      } catch (e) {
        onError(e);
      } finally {
        setDeletedLoading(false);
      }
    }
  }

  async function restore(
    key: string,
    action: () => Promise<void>,
    removeItem: (item: T) => boolean,
    after?: () => Promise<void> | void,
  ) {
    setRestoringKey(key);
    try {
      await action();
      setDeletedItems((prev) => prev.filter((item) => !removeItem(item)));
      await after?.();
    } catch (e) {
      onError(e);
    } finally {
      setRestoringKey(null);
    }
  }

  return {
    moreMode,
    toggleMoreMode,
    deletedItems,
    deletedLoading,
    restoringKey,
    restore,
  };
}
